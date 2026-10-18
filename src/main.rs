//! Glowing Cat Software Installer — entry point and UI wiring.
//!
//! Threading model: the Slint event loop owns the UI. All network work (release
//! lookups and downloads) runs on background `std::thread`s and posts results
//! back with `slint::invoke_from_event_loop` via a `Weak<MainWindow>`.

mod detect;
mod github;
mod install;
mod model;
mod platform;
mod selfinstall;

#[cfg(target_os = "macos")]
mod menu_macos;

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use platform::{Arch, InstallScope, PackageType};
use slint::{Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// Button/display state for a project, derived from release + install info.
/// Outcome of a release lookup (from cache or network), shared by both paths:
/// `(tag_name, assets as (name, url) pairs, no_release)`.
type ReleaseTriple = (Option<String>, Vec<(String, String)>, bool);

#[derive(Clone, Debug, Default)]
struct ProjectState {
    /// Locally recorded installed version, if installed.
    installed_version: Option<String>,
    /// Latest version string from GitHub, if a release was found.
    latest_version: Option<String>,
    /// Best matching asset (name, url) for the current OS/arch, if any.
    asset: Option<(String, String)>,
    /// A human-readable status/error line.
    status: String,
    /// True while a network op is in flight for this project.
    busy: bool,
    /// True if the repo has no release at all.
    no_release: bool,
}

/// Shared application state behind a mutex (touched by UI + worker threads).
struct AppState {
    pkg: PackageType,
    arch: Arch,
    scope: InstallScope,
    alt_held: bool,
    /// Per-project id -> state.
    projects: Vec<ProjectState>,
    install_state: install::InstallState,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // macOS: install our own winit backend with Slint's default menu bar
    // disabled. Otherwise Slint installs a default NSApp menu when the window
    // activates and hijacks muda's global menu-event handler, so clicks on our
    // native menu never reach us. Must happen before any window is created.
    #[cfg(target_os = "macos")]
    {
        let backend = i_slint_backend_winit::Backend::builder()
            .with_default_menu_bar(false)
            .build()?;
        slint::platform::set_platform(Box::new(backend))
            .expect("the Slint platform/backend must not be initialized yet");
    }

    let ui = MainWindow::new()?;

    let catalog = model::catalog();
    let icons = load_icons(&catalog);

    let host_pkg = PackageType::host_default();
    let host_arch = Arch::host();

    let app = Arc::new(Mutex::new(AppState {
        pkg: host_pkg,
        arch: host_arch,
        scope: InstallScope::ThisUser,
        alt_held: false,
        projects: vec![ProjectState::default(); catalog.len()],
        install_state: install::load_state(),
    }));

    // Seed installed versions from persisted state.
    {
        let mut st = app.lock().unwrap();
        for (i, p) in catalog.iter().enumerate() {
            st.projects[i].installed_version =
                install::installed_version(&st.install_state, p.repo);
        }
    }

    // --- Initialise static UI properties. ---
    let os_options: Vec<SharedString> = PackageType::all()
        .iter()
        .map(|p| SharedString::from(p.label()))
        .collect();
    ui.set_os_options(ModelRc::new(VecModel::from(os_options)));
    ui.set_os_index(host_pkg.index() as i32);
    ui.set_arch_label(SharedString::from(host_arch.label()));

    // Install scope selector ("Install for:"), defaulting to This User.
    let scope_options: Vec<SharedString> = InstallScope::all()
        .iter()
        .map(|s| SharedString::from(s.label()))
        .collect();
    ui.set_scope_options(ModelRc::new(VecModel::from(scope_options)));
    ui.set_scope_index(InstallScope::ThisUser.index() as i32);
    ui.set_dark_mode(true);
    ui.set_self_installed(selfinstall::is_installed());
    ui.set_self_install_hint(SharedString::from(selfinstall::install_location_hint()));
    ui.set_app_version(SharedString::from(env!("CARGO_PKG_VERSION")));

    // Windows/Linux use the Slint declarative menu bar; macOS uses the native
    // muda menu installed below, so the Slint one stays disabled there.
    #[cfg(not(target_os = "macos"))]
    ui.set_use_slint_menubar(true);

    // Application / window icon.
    if let Ok(icon) = slint::Image::load_from_path(&icons_dir().join("app_icon.png")) {
        ui.set_app_icon(icon);
    }

    // The project cards model, mutated as data arrives.
    let cards: Rc<VecModel<ProjectCard>> = Rc::new(VecModel::default());
    for (i, p) in catalog.iter().enumerate() {
        cards.push(ProjectCard {
            id: i as i32,
            name: SharedString::from(p.name),
            description: SharedString::from(p.description),
            icon: icons[i].clone(),
            action: SharedString::from("…"),
            status: SharedString::from("Checking…"),
            kind: SharedString::from("busy"),
            disabled: true,
        });
    }
    ui.set_projects(ModelRc::from(cards.clone()));

    // Catalog is read-only; share it cheaply.
    let catalog = Rc::new(catalog);

    // --- Callbacks ---

    // Theme toggle: mirror into Rust (nothing else needed; UI drives Theme).
    ui.on_theme_toggled({
        move |dark| {
            // Theme global is already bound to dark-mode in the UI; this hook
            // exists for future persistence of the preference.
            let _ = dark;
        }
    });

    // Alt/Option modifier: update shared state and refresh labels.
    {
        let app = app.clone();
        ui.on_alt_changed(move |held| {
            if let Ok(mut st) = app.lock() {
                st.alt_held = held;
            }
            // The UI updates Upgrade->Remove labels reactively via alt-held;
            // no model rewrite needed here.
        });
    }

    // Self-install this application onto the current machine.
    {
        let ui_weak = ui.as_weak();
        ui.on_self_install_pressed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                match selfinstall::install() {
                    Ok(path) => {
                        ui.set_self_installed(selfinstall::is_installed());
                        ui.set_self_install_hint(SharedString::from(path.clone()));
                        ui.set_global_status(SharedString::from(format!(
                            "Installed {} to {path}",
                            selfinstall::APP_DISPLAY_NAME
                        )));
                    }
                    Err(e) => {
                        ui.set_global_status(SharedString::from(format!(
                            "Self-install failed: {e}"
                        )));
                    }
                }
            }
        });
    }

    // Self-uninstall this application from the current machine.
    {
        let ui_weak = ui.as_weak();
        ui.on_self_uninstall_pressed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                match selfinstall::uninstall() {
                    Ok(()) => {
                        ui.set_self_installed(selfinstall::is_installed());
                        ui.set_self_install_hint(SharedString::from(
                            selfinstall::install_location_hint(),
                        ));
                        ui.set_global_status(SharedString::from(format!(
                            "Uninstalled {}",
                            selfinstall::APP_DISPLAY_NAME
                        )));
                    }
                    Err(e) => {
                        ui.set_global_status(SharedString::from(format!(
                            "Self-uninstall failed: {e}"
                        )));
                    }
                }
            }
        });
    }

    // About dialog: menu opens it; OK / scrim closes it; links open in the
    // system browser.
    {
        let ui_weak = ui.as_weak();
        ui.on_open_about(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_about(true);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_about_ok(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_about(false);
            }
        });
    }
    ui.on_open_url(|url| {
        open_url(&url);
    });

    // OS popup change: update target package type, re-resolve all projects.
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        let catalog = catalog.clone();
        let cards = cards.clone();
        ui.on_os_changed(move |idx| {
            {
                let mut st = app.lock().unwrap();
                st.pkg = PackageType::from_index(idx as usize);
            }
            refresh_all(&app, &catalog, &ui_weak, &cards, false);
        });
    }

    // Install-scope popup change: record it and re-evaluate every card against
    // the corresponding install location (e.g. macOS /Applications vs
    // ~/Applications), so the Install/Upgrade/Remove buttons reflect what is
    // installed in that scope.
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        let catalog = catalog.clone();
        let cards = cards.clone();
        ui.on_scope_changed(move |idx| {
            {
                let mut st = app.lock().unwrap();
                st.scope = InstallScope::from_index(idx as usize);
            }
            refresh_all(&app, &catalog, &ui_weak, &cards, false);
        });
    }

    // Manual refresh button.
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        let catalog = catalog.clone();
        let cards = cards.clone();
        ui.on_refresh(move || {
            // The explicit button forces a fresh GitHub check, bypassing the
            // hourly cache.
            refresh_all(&app, &catalog, &ui_weak, &cards, true);
        });
    }

    // Per-card action (Install / Upgrade / Remove).
    {
        let app = app.clone();
        let ui_weak = ui.as_weak();
        let catalog = catalog.clone();
        let cards = cards.clone();
        ui.on_action_pressed(move |id| {
            handle_action(id as usize, &app, &catalog, &ui_weak, &cards);
        });
    }

    // Kick off the initial release lookups (using the hourly cache).
    refresh_all(&app, &catalog, &ui.as_weak(), &cards, false);

    // macOS: install the native menu bar (Application menu with our About item,
    // plus standard File/Edit). The returned Menu must outlive the event loop,
    // so keep it bound until `run()` returns.
    #[cfg(target_os = "macos")]
    let _macos_menu = menu_macos::install(&ui);

    ui.run()?;
    Ok(())
}

/// Open a URL in the user's default browser via the OS, matching the
/// download-then-open approach used elsewhere (`install::open_installer`):
///   - macOS:   `open <url>`
///   - Windows: `cmd /C start "" <url>`
///   - Linux:   `xdg-open <url>`
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let (program, args): (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);

    let _ = std::process::Command::new(program).args(args).spawn();
}

/// Resolve the directory that contains `resources/icons`, independent of the
/// current working directory. We check, in order:
///   1. `<exe_dir>/resources/icons`               (binary shipped with resources)
///   2. `<exe_dir>/../Resources/resources/icons`  (inside a macOS .app bundle)
///   3. `<exe_dir>/../share/glowing-cat-installer/icons` (Linux .deb/.rpm FHS)
///   4. `/usr/share/glowing-cat-installer/icons`  (absolute FHS fallback)
///   5. `<cwd>/resources/icons`                   (e.g. `cargo run` from root)
///   6. `<CARGO_MANIFEST_DIR>/resources/icons`    (dev-time fallback)
///
/// Returns the first that exists, or the exe-relative path as a last resort.
fn icons_dir() -> std::path::PathBuf {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resources/icons"));
            candidates.push(dir.join("../Resources/resources/icons"));
            // Linux FHS layout from the .deb/.rpm: the binary lives in
            // <prefix>/bin and the icons in <prefix>/share/glowing-cat-installer/icons
            // (e.g. /usr/bin + /usr/share/glowing-cat-installer/icons).
            candidates.push(dir.join("../share/glowing-cat-installer/icons"));
        }
    }
    // Absolute FHS location, in case the exe path can't be resolved.
    candidates.push(std::path::PathBuf::from(
        "/usr/share/glowing-cat-installer/icons",
    ));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("resources/icons"));
    }
    candidates.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/icons"));

    for c in &candidates {
        if c.is_dir() {
            return c.clone();
        }
    }
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| std::path::PathBuf::from("resources/icons"))
}

/// Load each project's icon from the resolved icons dir, falling back to a
/// placeholder if a file is missing or fails to decode.
fn load_icons(catalog: &[model::Project]) -> Vec<slint::Image> {
    let base = icons_dir();
    let mut out = Vec::with_capacity(catalog.len());
    for p in catalog {
        let path = base.join(p.icon);
        let img = slint::Image::load_from_path(&path).unwrap_or_else(|_| placeholder_icon());
        out.push(img);
    }
    out
}

/// A simple blank placeholder used when an icon file is missing.
fn placeholder_icon() -> slint::Image {
    // 1x1 transparent pixel buffer; the UI image-fit keeps the slot sized.
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(2, 2);
    for px in buf.make_mut_slice() {
        *px = slint::Rgba8Pixel {
            r: 124,
            g: 92,
            b: 255,
            a: 40,
        };
    }
    slint::Image::from_rgba8(buf)
}

/// Recompute the UI card for one project id from its state.
fn apply_card(
    cards: &VecModel<ProjectCard>,
    id: usize,
    pstate: &ProjectState,
    name: &str,
    description: &str,
    icon: &slint::Image,
) {
    let installed = pstate.installed_version.clone();
    let (action, kind, disabled) = if pstate.busy {
        ("…".to_string(), "busy", true)
    } else if let Some(iv) = installed {
        // Installed (detected natively or recorded locally). Always offer at
        // least Remove, even if the latest release can't be determined right
        // now (network error / rate limit / no published release). Offer
        // Upgrade only when we positively know a strictly newer release exists.
        match &pstate.latest_version {
            Some(latest) if !iv.is_empty() && github::is_newer(&iv, latest) => {
                ("Upgrade".to_string(), "upgrade", false)
            }
            _ => ("Remove".to_string(), "remove", false),
        }
    } else if pstate.no_release {
        // Not installed and the repo has no published release.
        ("Unavailable".to_string(), "none", true)
    } else if pstate.asset.is_some() {
        // Not installed, a matching asset exists.
        ("Install".to_string(), "install", false)
    } else if pstate.latest_version.is_some() {
        // A release exists but no asset matches this OS/arch.
        ("No build".to_string(), "none", true)
    } else {
        // No info yet (e.g. network lookup failed and not installed).
        ("Unavailable".to_string(), "none", true)
    };

    cards.set_row_data(
        id,
        ProjectCard {
            id: id as i32,
            name: SharedString::from(name),
            description: SharedString::from(description),
            icon: icon.clone(),
            action: SharedString::from(action),
            status: SharedString::from(pstate.status.clone()),
            kind: SharedString::from(kind),
            disabled,
        },
    );
}

/// Spawn background lookups for every project.
fn refresh_all(
    app: &Arc<Mutex<AppState>>,
    catalog: &Rc<Vec<model::Project>>,
    ui_weak: &slint::Weak<MainWindow>,
    cards: &Rc<VecModel<ProjectCard>>,
    force: bool,
) {
    let (pkg, arch, scope) = {
        let st = app.lock().unwrap();
        (st.pkg, st.arch, st.scope)
    };

    for (i, project) in catalog.iter().enumerate() {
        // Mark busy immediately.
        {
            let mut st = app.lock().unwrap();
            st.projects[i].busy = true;
            st.projects[i].status = "Checking…".to_string();
        }
        push_card(app, catalog, ui_weak, cards, i);

        let slug = project.slug();
        let app = app.clone();
        let ui_weak = ui_weak.clone();
        let catalog_names: (String, String) =
            (project.name.to_string(), project.description.to_string());
        // Clone the project so the worker can run OS-native install detection.
        let project_for_detect = project.clone();

        thread::spawn(move || {
            // 1. Native detection first: does the OS already have this installed?
            //    This runs quick local subprocesses (dpkg/rpm/defaults/reg) and
            //    reflects reality even for software installed outside this app.
            // 1. Native detection first: does the OS already have this installed?
            //    This runs quick local subprocesses (dpkg/rpm/defaults/reg) and
            //    reflects reality even for software installed outside this app.
            //    We also pass the exact path we recorded at install time (if
            //    any), so detection is robust to real bundle names that differ
            //    from the catalog's expected name.
            let recorded_path = {
                let st = app.lock().unwrap();
                install::installed_path(&st.install_state, project_for_detect.repo)
                    .map(|s| s.to_string())
            };
            let native_version = detect::detect_installed_version(
                &project_for_detect,
                pkg,
                scope,
                recorded_path.as_deref(),
            );

            // 2. Release info: use the hourly cache if fresh, else fetch from
            //    GitHub and write the result back into installed.json.
            //    `triple` = (tag_name, assets as (name,url) pairs, no_release).
            let repo_key = project_for_detect.repo.to_string();
            let cached = if force {
                None // explicit Refresh bypasses the hourly cache
            } else {
                let st = app.lock().unwrap();
                st.install_state.fresh_cache(&repo_key).map(|c| {
                    (
                        c.tag_name.clone(),
                        c.assets
                            .iter()
                            .map(|a| (a.name.clone(), a.url.clone()))
                            .collect::<Vec<_>>(),
                        c.no_release,
                    )
                })
            };

            let triple: Option<ReleaseTriple> = match cached {
                Some(c) => Some(c), // cache hit — no network call
                None => {
                    // Cache miss or stale: hit the network, then persist.
                    match github::latest_release(&slug) {
                        Ok(rel) => {
                            let assets: Vec<(String, String)> = rel
                                .assets
                                .iter()
                                .map(|a| (a.name.clone(), a.download_url.clone()))
                                .collect();
                            let cached_assets: Vec<install::CachedAsset> = assets
                                .iter()
                                .map(|(n, u)| install::CachedAsset {
                                    name: n.clone(),
                                    url: u.clone(),
                                })
                                .collect();
                            let mut st = app.lock().unwrap();
                            st.install_state.put_cache(
                                &repo_key,
                                Some(rel.tag_name.clone()),
                                cached_assets,
                                false,
                            );
                            let _ = install::save_state(&st.install_state);
                            Some((Some(rel.tag_name.clone()), assets, false))
                        }
                        Err(github::FetchError::NoRelease) => {
                            let mut st = app.lock().unwrap();
                            st.install_state.put_cache(&repo_key, None, Vec::new(), true);
                            let _ = install::save_state(&st.install_state);
                            Some((None, Vec::new(), true))
                        }
                        Err(github::FetchError::Http(_)) => None, // leave unknown; don't cache
                    }
                }
            };

            // 3. Apply to project state.
            {
                let mut st = app.lock().unwrap();
                let ps = &mut st.projects[i];
                ps.busy = false;

                // Merge native detection with the local installed.json record.
                //
                // On macOS the install location depends on the selected scope
                // (/Applications vs ~/Applications), and native detection is
                // scope-aware, so it is authoritative: if the OS does not find
                // the app in the selected scope, treat it as not installed even
                // if installed.json has a record (it may have been installed in
                // the other scope). On Linux/Windows scope doesn't change
                // detection, so a positive native result wins but a negative one
                // leaves the local record intact.
                if pkg == PackageType::Macos {
                    ps.installed_version = native_version.clone();
                } else if let Some(nv) = native_version.clone() {
                    ps.installed_version = Some(nv);
                }

                match triple {
                    Some((Some(tag), assets, false)) => {
                        ps.no_release = false;
                        ps.latest_version = Some(tag.clone());
                        match github::best_named_asset(&assets, pkg, arch) {
                            Some((name, url)) => {
                                ps.asset = Some((name.clone(), url.clone()));
                                ps.status = match &ps.installed_version {
                                    Some(v) if v.is_empty() => {
                                        format!("Installed · latest {tag}")
                                    }
                                    Some(v) if github::is_newer(v, &tag) => {
                                        format!("Installed {v} · latest {tag}")
                                    }
                                    Some(v) => format!("Installed {v} (up to date)"),
                                    None => format!("Latest {tag}"),
                                };
                            }
                            None => {
                                ps.asset = None;
                                ps.status =
                                    format!("No {} build for {}", pkg.label(), arch.label());
                            }
                        }
                    }
                    Some((_, _, true)) | Some((None, _, _)) => {
                        // No published release.
                        ps.no_release = true;
                        ps.latest_version = None;
                        ps.asset = None;
                        ps.status = "No published release".to_string();
                    }
                    None => {
                        // Network error and no fresh cache: leave release info
                        // unknown. Installed apps still show Remove via apply_card.
                        ps.asset = None;
                        if ps.installed_version.is_none() {
                            ps.status = "Version check unavailable".to_string();
                        }
                    }
                }
            }

            // Repaint this card on the UI thread.
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let model = ui.get_projects();
                    if let Some(cards) = model
                        .as_any()
                        .downcast_ref::<VecModel<ProjectCard>>()
                    {
                        // Reconstruct the pstate snapshot for apply_card.
                        if let Ok(st) = app.lock() {
                            let ps = &st.projects[i];
                            let icon = cards
                                .row_data(i)
                                .map(|c| c.icon)
                                .unwrap_or_else(placeholder_icon);
                            apply_card(
                                cards,
                                i,
                                ps,
                                &catalog_names.0,
                                &catalog_names.1,
                                &icon,
                            );
                        }
                    }
                }
            });
        });
    }
}

/// Repaint a single card from current state (synchronously, UI thread).
fn push_card(
    app: &Arc<Mutex<AppState>>,
    catalog: &Rc<Vec<model::Project>>,
    _ui_weak: &slint::Weak<MainWindow>,
    cards: &Rc<VecModel<ProjectCard>>,
    id: usize,
) {
    let st = app.lock().unwrap();
    let ps = &st.projects[id];
    let p = &catalog[id];
    let icon = cards
        .row_data(id)
        .map(|c| c.icon)
        .unwrap_or_else(placeholder_icon);
    apply_card(cards, id, ps, p.name, p.description, &icon);
}

/// Handle an Install/Upgrade/Remove button press.
fn handle_action(
    id: usize,
    app: &Arc<Mutex<AppState>>,
    catalog: &Rc<Vec<model::Project>>,
    ui_weak: &slint::Weak<MainWindow>,
    cards: &Rc<VecModel<ProjectCard>>,
) {
    // Decide the action from current state + Alt modifier.
    enum Action {
        Install { asset: (String, String), version: String },
        Remove,
        None,
    }

    let (pkg, arch, action, repo) = {
        let st = app.lock().unwrap();
        let ps = &st.projects[id];
        let repo = catalog[id].repo.to_string();

        if ps.busy {
            (st.pkg, st.arch, Action::None, repo)
        } else if let Some(installed) = ps.installed_version.clone() {
            // Installed (native or local). Upgrade only when a strictly newer
            // release is positively known; otherwise the action is Remove
            // (also when Alt is held over an Upgrade).
            let upgradable = !installed.is_empty()
                && ps
                    .latest_version
                    .as_ref()
                    .map(|l| github::is_newer(&installed, l))
                    .unwrap_or(false);

            if upgradable && !st.alt_held {
                match &ps.asset {
                    Some(a) => (
                        st.pkg,
                        st.arch,
                        Action::Install {
                            asset: a.clone(),
                            version: ps.latest_version.clone().unwrap_or_default(),
                        },
                        repo,
                    ),
                    None => (st.pkg, st.arch, Action::None, repo),
                }
            } else {
                (st.pkg, st.arch, Action::Remove, repo)
            }
        } else if ps.no_release {
            // Not installed and nothing to install.
            (st.pkg, st.arch, Action::None, repo)
        } else {
            // Not installed: Install if we have a matching asset.
            match &ps.asset {
                Some(a) => (
                    st.pkg,
                    st.arch,
                    Action::Install {
                        asset: a.clone(),
                        version: ps.latest_version.clone().unwrap_or_default(),
                    },
                    repo,
                ),
                None => (st.pkg, st.arch, Action::None, repo),
            }
        }
    };

    match action {
        Action::None => {}
        Action::Remove => {
            let project = catalog[id].clone();
            let app_name = project.macos_app_name();
            let pkg_name = project.deb_rpm_package();

            // Run the automated uninstall (delete the macOS .app, apt/dnf
            // remove on Linux, registry UninstallString on Windows) and clear
            // our local tracking.
            let remove_result = {
                let mut st = app.lock().unwrap();
                let pkg = st.pkg;
                install::remove_project(&mut st.install_state, &repo, pkg, &app_name, &pkg_name)
            };

            match remove_result {
                Err(e) => {
                    let mut st = app.lock().unwrap();
                    st.projects[id].status = format!("Remove failed: {e}");
                }
                Ok(_performed) => {
                    // Re-detect to confirm the software is actually gone. The
                    // local record was just cleared by remove_project, so pass
                    // None and rely on the scope's Applications folder.
                    let still_present = {
                        let st = app.lock().unwrap();
                        detect::detect_installed_version(&project, st.pkg, st.scope, None)
                    };
                    let mut st = app.lock().unwrap();
                    match still_present {
                        Some(_) => {
                            // Could not fully remove it (e.g. a copy in a
                            // root-owned /Applications, or no pkexec): keep it
                            // marked installed and point at where to remove it.
                            let hint = detect::install_location_hint(&project, st.pkg, st.scope);
                            st.projects[id].status = format!(
                                "Installed outside this app — remove via the OS ({hint})"
                            );
                        }
                        None => {
                            st.projects[id].installed_version = None;
                            st.projects[id].status = "Removed".to_string();
                        }
                    }
                }
            }
            push_card(app, catalog, ui_weak, cards, id);
        }
        Action::Install { asset, version } => {
            // Mark busy, then do the download+open on a worker thread.
            {
                let mut st = app.lock().unwrap();
                st.projects[id].busy = true;
                st.projects[id].status = "Downloading…".to_string();
            }
            push_card(app, catalog, ui_weak, cards, id);

            // Read the current install scope ("All Users" / "This User").
            let scope = app.lock().map(|st| st.scope).unwrap_or(InstallScope::ThisUser);

            let app = app.clone();
            let ui_weak = ui_weak.clone();
            let repo2 = repo.clone();
            let (asset_name, asset_url) = asset;
            // Names needed by the automated installer (macOS .app bundle,
            // dpkg/rpm package).
            let app_name = catalog[id].macos_app_name();
            let pkg_name = catalog[id].deb_rpm_package();
            thread::spawn(move || {
                let res = {
                    let mut st = app.lock().unwrap();
                    install::install_project(
                        &mut st.install_state,
                        &repo2,
                        &version,
                        pkg,
                        arch,
                        scope,
                        &asset_name,
                        &asset_url,
                        &app_name,
                        &pkg_name,
                    )
                };
                {
                    let mut st = app.lock().unwrap();
                    let ps = &mut st.projects[id];
                    ps.busy = false;
                    match res {
                        Ok(outcome) => {
                            ps.installed_version = Some(version.clone());
                            ps.status = if outcome.automated {
                                "Installed".to_string()
                            } else {
                                // Fallback path: the OS/native installer is
                                // handling the rest.
                                "Running installer…".to_string()
                            };
                        }
                        Err(e) => {
                            ps.status = format!("Failed: {e}");
                        }
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        let model = ui.get_projects();
                        if let Some(cards) =
                            model.as_any().downcast_ref::<VecModel<ProjectCard>>()
                        {
                            if let Ok(st) = app.lock() {
                                let ps = &st.projects[id];
                                // We need catalog name/desc; recover from the card.
                                if let Some(existing) = cards.row_data(id) {
                                    apply_card(
                                        cards,
                                        id,
                                        ps,
                                        existing.name.as_ref(),
                                        existing.description.as_ref(),
                                        &existing.icon,
                                    );
                                }
                            }
                        }
                    }
                });
            });
        }
    }
}
