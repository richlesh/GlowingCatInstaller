//! Download-then-open installation flow, installed-state tracking, and removal.
//!
//! ## v1 behaviour (download-then-open)
//! Pressing **Install**/**Upgrade** downloads the matching release asset to the
//! user's Downloads directory (or the app cache dir as a fallback) and then
//! reveals/opens it with the OS so the user can complete installation with the
//! native installer. We then record the installed version in a local state file.
//!
//! ## Path to full automation
//! The `open_installer` function is the single seam where silent installation
//! would plug in. See its doc comment for the exact per-OS commands that a
//! future version would run (with privilege elevation). Everything else —
//! asset selection, download, state tracking, removal — is already in place.

use crate::platform::{Arch, InstallScope, PackageType};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs, UserDirs};

/// Record of an installed project, keyed by repo slug in the state file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledEntry {
    /// Version string we installed (the release tag, normalised minimally).
    pub version: String,
    /// Which package type was installed.
    pub pkg: String,
    /// Which arch was installed.
    pub arch: String,
    /// The asset filename that was downloaded.
    pub asset: String,
    /// Where we saved the downloaded installer (may no longer exist).
    pub downloaded_to: String,
    /// Where the software was actually installed, so uninstall can reverse it.
    /// - macOS: the absolute path to the copied `<App>.app` bundle.
    /// - Linux: the dpkg/rpm package name that was installed.
    /// - Windows: empty (uninstall uses the registry UninstallString).
    ///
    /// Empty when the install used the download-then-open fallback (nothing to
    /// automatically reverse).
    #[serde(default)]
    pub installed_to: String,
}

/// A cached release asset (name + download URL), mirroring github::Asset but
/// serialisable into the state file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAsset {
    pub name: String,
    pub url: String,
}

/// Cached result of a GitHub release lookup for one repo, with the time it was
/// fetched. Stored so we only hit the GitHub API at most once per hour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRelease {
    /// Unix timestamp (seconds) of when this lookup was performed.
    pub checked_at: u64,
    /// The release tag, or `None` if the repo has no published release.
    #[serde(default)]
    pub tag_name: Option<String>,
    /// All assets from the release (empty when `no_release`).
    #[serde(default)]
    pub assets: Vec<CachedAsset>,
    /// True if the repo has no published release at all.
    #[serde(default)]
    pub no_release: bool,
}

/// Time-to-live for a cached release lookup: 1 hour.
pub const CACHE_TTL_SECS: u64 = 3600;

/// Current unix time in seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// On-disk persisted install state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallState {
    /// repo-name -> entry
    #[serde(default)]
    pub installed: BTreeMap<String, InstalledEntry>,
    /// repo-name -> cached GitHub release lookup (hourly TTL).
    #[serde(default)]
    pub release_cache: BTreeMap<String, CachedRelease>,
}

impl InstallState {
    /// Return the cached release for `repo` if it is still fresh (younger than
    /// `CACHE_TTL_SECS`); otherwise `None` so the caller fetches anew.
    pub fn fresh_cache(&self, repo: &str) -> Option<&CachedRelease> {
        let entry = self.release_cache.get(repo)?;
        let age = now_secs().saturating_sub(entry.checked_at);
        if age < CACHE_TTL_SECS {
            Some(entry)
        } else {
            None
        }
    }

    /// Insert / replace the cached release for `repo`, stamped with now().
    pub fn put_cache(
        &mut self,
        repo: &str,
        tag_name: Option<String>,
        assets: Vec<CachedAsset>,
        no_release: bool,
    ) {
        self.release_cache.insert(
            repo.to_string(),
            CachedRelease {
                checked_at: now_secs(),
                tag_name,
                assets,
                no_release,
            },
        );
    }
}

/// Errors from install operations.
#[derive(Debug)]
pub enum InstallError {
    Io(String),
    Http(String),
    Launch(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Io(e) => write!(f, "file error: {e}"),
            InstallError::Http(e) => write!(f, "download error: {e}"),
            InstallError::Launch(e) => write!(f, "could not open installer: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        InstallError::Io(e.to_string())
    }
}

/// Resolve the application's data directory, creating it if needed.
fn data_dir() -> PathBuf {
    if let Some(pd) = ProjectDirs::from("cloud", "GlowingCat", "GlowingCatInstaller") {
        let dir = pd.data_dir().to_path_buf();
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    // Fallback: a dot-dir in the home directory.
    let base = BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(".glowing-cat-installer");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Directory to save downloaded installers into (Downloads, else cache).
fn download_dir() -> PathBuf {
    if let Some(ud) = UserDirs::new() {
        if let Some(dl) = ud.download_dir() {
            if dl.exists() {
                return dl.to_path_buf();
            }
        }
    }
    let dir = data_dir().join("downloads");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn state_path() -> PathBuf {
    data_dir().join("installed.json")
}

/// Load install state from disk (empty if none / unreadable).
pub fn load_state() -> InstallState {
    let path = state_path();
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => InstallState::default(),
    }
}

/// Persist install state to disk.
pub fn save_state(state: &InstallState) -> Result<(), InstallError> {
    let path = state_path();
    let json = serde_json::to_string_pretty(state).map_err(|e| InstallError::Io(e.to_string()))?;
    let mut f = fs::File::create(&path)?;
    f.write_all(json.as_bytes())?;
    Ok(())
}

/// The locally-recorded installed version for a repo, if any.
pub fn installed_version(state: &InstallState, repo: &str) -> Option<String> {
    state.installed.get(repo).map(|e| e.version.clone())
}

/// The exact location this app recorded installing a repo to, if any (macOS:
/// the `.app` bundle path). Empty strings are treated as "not recorded".
pub fn installed_path<'a>(state: &'a InstallState, repo: &str) -> Option<&'a str> {
    state
        .installed
        .get(repo)
        .map(|e| e.installed_to.as_str())
        .filter(|s| !s.is_empty())
}

/// Download an asset to the downloads directory, returning the saved path.
pub fn download_asset(url: &str, filename: &str) -> Result<PathBuf, InstallError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("GlowingCatInstaller/0.1 (+https://github.com/richlesh)")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| InstallError::Http(e.to_string()))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| InstallError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(InstallError::Http(format!("HTTP {}", resp.status())));
    }

    let bytes = resp.bytes().map_err(|e| InstallError::Http(e.to_string()))?;

    let dest = download_dir().join(filename);
    let mut f = fs::File::create(&dest)?;
    f.write_all(&bytes)?;
    Ok(dest)
}

/// Result of performing an installation: where the software landed (so uninstall
/// can reverse it) and whether it was a true automated install or the
/// download-then-open fallback.
pub struct InstallOutcome {
    /// Install location recorded in state (`installed_to`): the `.app` path on
    /// macOS, the package name on Linux, or empty on Windows / fallback.
    pub installed_to: String,
    /// False when we had to fall back to handing the file to the OS (e.g. a
    /// `.pkg`, an unknown `.exe`, or a missing `pkexec`).
    pub automated: bool,
}

/// Run a command to completion and require success, capturing stderr on failure.
fn run(program: &str, args: &[&str]) -> Result<(), InstallError> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| InstallError::Launch(format!("{program}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = msg.trim();
        Err(InstallError::Launch(if msg.is_empty() {
            format!("{program} exited with {}", out.status)
        } else {
            format!("{program}: {msg}")
        }))
    }
}

/// Spawn a command without waiting (for the download-then-open fallback, where
/// the OS/installer owns the rest of the flow).
fn spawn(program: &str, args: &[&str]) -> Result<(), InstallError> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| InstallError::Launch(format!("{program}: {e}")))
}

/// Hand the downloaded file to the OS so the user completes installation with
/// the native installer. Used as a fallback when an automated path isn't
/// available (`.pkg`, unknown `.exe`, missing `pkexec`, unsupported OS).
fn open_with_os(path: &Path) -> Result<(), InstallError> {
    let p = path.to_string_lossy().to_string();
    #[cfg(target_os = "macos")]
    {
        spawn("open", &[&p])
    }
    #[cfg(target_os = "windows")]
    {
        // `start` is a cmd builtin; the empty "" is the window title arg.
        spawn("cmd", &["/C", "start", "", &p])
    }
    #[cfg(target_os = "linux")]
    {
        spawn("xdg-open", &[&p])
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = &p;
        Err(InstallError::Launch("unsupported OS".into()))
    }
}

/// Install the downloaded asset automatically, per OS. `app_name` is the macOS
/// `.app` bundle name (no extension); `pkg_name` is the dpkg/rpm package name.
///
/// Behaviour (user-scoped where possible; elevated only where the OS requires):
///   - **macOS .dmg** → mount with `hdiutil`, copy the `.app` to `~/Applications`
///     (no admin needed), detach. `.pkg` falls back to `open`.
///   - **Linux .deb** → `pkexec apt-get install -y <file>`;
///     **.rpm** → `pkexec dnf install -y <file>` (polkit shows a GUI prompt).
///     Falls back to download-then-open if `pkexec` is unavailable.
///   - **Windows .msi** → `msiexec /i <file> /qb`; **.exe** → launch the
///     installer (silent flags vary by installer; UAC handles elevation).
fn perform_install(
    path: &Path,
    pkg: PackageType,
    scope: InstallScope,
    app_name: &str,
    pkg_name: &str,
) -> Result<InstallOutcome, InstallError> {
    let _ = (app_name, pkg_name); // not all are used on every OS
    match pkg {
        PackageType::Macos => install_macos(path, scope, app_name),
        PackageType::Deb => install_linux(path, pkg_name, LinuxPm::Apt),
        PackageType::Rpm => install_linux(path, pkg_name, LinuxPm::Dnf),
        PackageType::Windows => install_windows(path, scope),
    }
}

/// macOS: mount the DMG, copy the first `.app` found into the Applications
/// folder for the chosen scope (`/Applications` for All Users — with an admin
/// prompt — or `~/Applications` for This User), then detach. `.pkg` (or
/// anything that isn't a DMG we can read) falls back to `open`.
fn install_macos(
    path: &Path,
    scope: InstallScope,
    app_name: &str,
) -> Result<InstallOutcome, InstallError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    if ext != "dmg" {
        // .pkg and friends: let the OS run the native installer.
        open_with_os(path)?;
        return Ok(InstallOutcome { installed_to: String::new(), automated: false });
    }

    // Mount read-only without opening a Finder window; parse the mount point.
    let p = path.to_string_lossy().to_string();
    let out = std::process::Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-noautoopen", "-readonly", &p])
        .output()
        .map_err(|e| InstallError::Launch(format!("hdiutil: {e}")))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(InstallError::Launch(format!("hdiutil attach: {}", msg.trim())));
    }
    // The last whitespace-separated token on each line is the mount path; the
    // volume mount lives under /Volumes.
    let listing = String::from_utf8_lossy(&out.stdout);
    let mount_point = listing
        .lines()
        .filter_map(|l| l.split('\t').next_back().map(str::trim))
        .find(|m| m.starts_with("/Volumes/"))
        .map(PathBuf::from);

    let Some(mount_point) = mount_point else {
        return Err(InstallError::Launch(
            "could not determine the mounted DMG volume".into(),
        ));
    };

    // Ensure we always detach, even on error.
    let result = (|| -> Result<String, InstallError> {
        // Prefer the expected bundle name; otherwise take the first .app.
        let preferred = mount_point.join(format!("{app_name}.app"));
        let app_src = if preferred.exists() {
            preferred
        } else {
            find_first_app(&mount_point)
                .ok_or_else(|| InstallError::Launch("no .app bundle inside the DMG".into()))?
        };
        let app_file_name = app_src
            .file_name()
            .ok_or_else(|| InstallError::Launch("bad .app path".into()))?;

        let dest_dir = applications_dir(scope);
        let dest = dest_dir.join(app_file_name);

        match scope {
            InstallScope::ThisUser => {
                // ~/Applications is user-writable: copy directly.
                fs::create_dir_all(&dest_dir)?;
                if dest.exists() {
                    let _ = fs::remove_dir_all(&dest);
                }
                // ditto preserves bundle metadata/permissions.
                run("ditto", &[&app_src.to_string_lossy(), &dest.to_string_lossy()])?;
            }
            InstallScope::AllUsers => {
                // /Applications is root-owned: run the replace+copy as one
                // elevated shell via an admin prompt (osascript). Quote paths
                // for the shell inside AppleScript.
                let src = app_src.to_string_lossy().replace('\'', "'\\''");
                let dst = dest.to_string_lossy().replace('\'', "'\\''");
                let shell_cmd = format!("rm -rf '{dst}' && /usr/bin/ditto '{src}' '{dst}'");
                run_with_admin(&shell_cmd)?;
            }
        }
        Ok(dest.to_string_lossy().to_string())
    })();

    // Detach regardless of success.
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", &mount_point.to_string_lossy(), "-quiet"])
        .output();

    let installed_to = result?;
    Ok(InstallOutcome { installed_to, automated: true })
}

/// Find the first `*.app` directory at the top level of a mounted volume.
fn find_first_app(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("app") {
            return Some(p);
        }
    }
    None
}

/// The Applications directory for the given scope: `/Applications` (system,
/// all users) or `~/Applications` (this user).
fn applications_dir(scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::AllUsers => PathBuf::from("/Applications"),
        InstallScope::ThisUser => BaseDirs::new()
            .map(|b| b.home_dir().join("Applications"))
            .unwrap_or_else(|| PathBuf::from("Applications")),
    }
}

/// Run a shell command with macOS administrator privileges via an osascript
/// prompt (the standard GUI way to get a one-off admin action). The user sees a
/// native password dialog; cancelling yields an error.
#[cfg(target_os = "macos")]
fn run_with_admin(shell_cmd: &str) -> Result<(), InstallError> {
    // Escape double quotes and backslashes for embedding in the AppleScript
    // string literal.
    let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{escaped}\" with administrator privileges");
    run("osascript", &["-e", &script])
}
#[cfg(not(target_os = "macos"))]
fn run_with_admin(_shell_cmd: &str) -> Result<(), InstallError> {
    Err(InstallError::Launch("admin elevation not supported".into()))
}

enum LinuxPm {
    Apt,
    Dnf,
}

/// Linux: install a local package file via the system package manager with a
/// graphical privilege prompt (`pkexec`). `apt-get install` / `dnf install` on
/// a local path resolve dependencies (unlike `dpkg -i` / `rpm -i`). Falls back
/// to download-then-open when `pkexec` isn't available.
fn install_linux(
    path: &Path,
    pkg_name: &str,
    pm: LinuxPm,
) -> Result<InstallOutcome, InstallError> {
    if !command_exists("pkexec") {
        open_with_os(path)?;
        return Ok(InstallOutcome { installed_to: String::new(), automated: false });
    }
    let p = path.to_string_lossy().to_string();
    match pm {
        LinuxPm::Apt => run("pkexec", &["apt-get", "install", "-y", &p])?,
        LinuxPm::Dnf => run("pkexec", &["dnf", "install", "-y", &p])?,
    }
    Ok(InstallOutcome {
        installed_to: pkg_name.to_string(),
        automated: true,
    })
}

/// Windows: run the installer. `.msi` installs with a basic-UI progress dialog
/// (`msiexec /i /qb`), per-machine (`ALLUSERS=1`, UAC prompt) or per-user
/// (`MSIINSTALLPERUSER=1`, no elevation) depending on scope; `.exe` is launched
/// directly (silent/scope flags vary between NSIS/Inno/etc., so we let the
/// installer's own UI drive). Uninstall is driven from the registry
/// UninstallString, so no `installed_to` is recorded.
fn install_windows(path: &Path, scope: InstallScope) -> Result<InstallOutcome, InstallError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let p = path.to_string_lossy().to_string();
    if ext == "msi" {
        match scope {
            InstallScope::AllUsers => run("msiexec", &["/i", &p, "/qb", "ALLUSERS=1"])?,
            InstallScope::ThisUser => {
                run("msiexec", &["/i", &p, "/qb", "MSIINSTALLPERUSER=1", "ALLUSERS="])?
            }
        }
        Ok(InstallOutcome { installed_to: String::new(), automated: true })
    } else {
        // .exe (or unknown): launch the installer; it drives the rest.
        spawn("cmd", &["/C", "start", "", &p])?;
        Ok(InstallOutcome { installed_to: String::new(), automated: false })
    }
}

/// Whether a program is on PATH (Unix `command -v`).
#[cfg(unix)]
fn command_exists(program: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
#[cfg(not(unix))]
fn command_exists(_program: &str) -> bool {
    false
}

/// Full install/upgrade flow: download the asset, install it automatically, and
/// record state (including where it landed so uninstall can reverse it).
#[allow(clippy::too_many_arguments)]
pub fn install_project(
    state: &mut InstallState,
    repo: &str,
    version: &str,
    pkg: PackageType,
    arch: Arch,
    scope: InstallScope,
    asset_name: &str,
    asset_url: &str,
    app_name: &str,
    pkg_name: &str,
) -> Result<InstallOutcome, InstallError> {
    let path = download_asset(asset_url, asset_name)?;
    let outcome = perform_install(&path, pkg, scope, app_name, pkg_name)?;

    state.installed.insert(
        repo.to_string(),
        InstalledEntry {
            version: version.to_string(),
            pkg: format!("{pkg:?}"),
            arch: arch.label().to_string(),
            asset: asset_name.to_string(),
            downloaded_to: path.to_string_lossy().to_string(),
            installed_to: outcome.installed_to.clone(),
        },
    );
    save_state(state)?;
    Ok(outcome)
}

/// Automatically uninstall a project, per OS, then clear local tracking.
///
///   - **macOS** → delete the recorded `~/Applications/<App>.app` bundle.
///   - **Linux** → `pkexec apt-get remove -y <pkg>` / `dnf remove -y <pkg>`.
///   - **Windows** → run the registry UninstallString (handled in `detect`).
///
/// `app_name`/`pkg_name` provide a fallback target when the stored
/// `installed_to` is empty (e.g. installs recorded before this field existed).
/// Returns `true` when a system-level uninstall was actually performed.
pub fn remove_project(
    state: &mut InstallState,
    repo: &str,
    pkg: PackageType,
    app_name: &str,
    pkg_name: &str,
) -> Result<bool, InstallError> {
    let entry = state.installed.remove(repo);

    // Delete the cached installer file if we still have it.
    if let Some(e) = &entry {
        let p = PathBuf::from(&e.downloaded_to);
        if p.exists() {
            let _ = fs::remove_file(&p);
        }
    }

    let installed_to = entry.as_ref().map(|e| e.installed_to.clone()).unwrap_or_default();
    let performed = perform_uninstall(pkg, &installed_to, app_name, pkg_name)?;

    save_state(state)?;
    Ok(performed)
}

/// Reverse an install per OS. Returns `true` if a real uninstall ran.
fn perform_uninstall(
    pkg: PackageType,
    installed_to: &str,
    app_name: &str,
    pkg_name: &str,
) -> Result<bool, InstallError> {
    match pkg {
        PackageType::Macos => {
            // Prefer the exact recorded bundle path; else try ~/Applications
            // then /Applications by convention.
            let candidates: Vec<PathBuf> = if !installed_to.is_empty() {
                vec![PathBuf::from(installed_to)]
            } else {
                vec![
                    applications_dir(InstallScope::ThisUser).join(format!("{app_name}.app")),
                    applications_dir(InstallScope::AllUsers).join(format!("{app_name}.app")),
                ]
            };
            for app in candidates {
                if app.exists() {
                    // ~/Applications is user-writable (no admin). A bundle under
                    // /Applications is root-owned, so if a plain remove fails
                    // there, retry with an admin prompt before giving up.
                    match fs::remove_dir_all(&app) {
                        Ok(()) => return Ok(true),
                        Err(_) if app.starts_with("/Applications") => {
                            let dst = app.to_string_lossy().replace('\'', "'\\''");
                            run_with_admin(&format!("rm -rf '{dst}'"))?;
                            return Ok(true);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            Ok(false)
        }
        PackageType::Deb | PackageType::Rpm => {
            let name = if !installed_to.is_empty() { installed_to } else { pkg_name };
            if name.is_empty() || !command_exists("pkexec") {
                return Ok(false);
            }
            match pkg {
                PackageType::Deb => run("pkexec", &["apt-get", "remove", "-y", name])?,
                _ => run("pkexec", &["dnf", "remove", "-y", name])?,
            }
            Ok(true)
        }
        PackageType::Windows => {
            // Driven from the registry UninstallString.
            crate::detect::run_windows_uninstall(app_name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip_serde() {
        let mut s = InstallState::default();
        s.installed.insert(
            "SpaceServal".to_string(),
            InstalledEntry {
                version: "1.1.0".into(),
                pkg: "Macos".into(),
                arch: "arm64".into(),
                asset: "SpaceServal-aarch64-apple-darwin.dmg".into(),
                downloaded_to: "/tmp/x.dmg".into(),
                installed_to: String::new(),
            },
        );
        let json = serde_json::to_string(&s).unwrap();
        let back: InstallState = serde_json::from_str(&json).unwrap();
        assert_eq!(installed_version(&back, "SpaceServal").as_deref(), Some("1.1.0"));
        assert!(installed_version(&back, "Nope").is_none());
    }

    #[test]
    fn empty_state_from_garbage() {
        let s: InstallState = serde_json::from_str("not json").unwrap_or_default();
        assert!(s.installed.is_empty());
    }

    #[test]
    fn cache_freshness_ttl() {
        let mut s = InstallState::default();
        s.put_cache(
            "SpaceServal",
            Some("v1.1.0".into()),
            vec![CachedAsset {
                name: "SpaceServal-aarch64-apple-darwin.dmg".into(),
                url: "https://example/x.dmg".into(),
            }],
            false,
        );
        // Just-written entry is fresh.
        assert!(s.fresh_cache("SpaceServal").is_some());
        // A never-written repo has no cache.
        assert!(s.fresh_cache("Nope").is_none());

        // Backdate the timestamp beyond the TTL -> no longer fresh.
        if let Some(e) = s.release_cache.get_mut("SpaceServal") {
            e.checked_at = now_secs().saturating_sub(CACHE_TTL_SECS + 10);
        }
        assert!(s.fresh_cache("SpaceServal").is_none());

        // But the data is still present (so a forced refresh can replace it).
        assert!(s.release_cache.contains_key("SpaceServal"));
    }

    #[test]
    fn cache_survives_serde_roundtrip() {
        let mut s = InstallState::default();
        s.put_cache("TagTiger", Some("v1.2.0".into()), Vec::new(), false);
        let json = serde_json::to_string(&s).unwrap();
        let back: InstallState = serde_json::from_str(&json).unwrap();
        assert!(back.release_cache.contains_key("TagTiger"));
        assert_eq!(
            back.release_cache["TagTiger"].tag_name.as_deref(),
            Some("v1.2.0")
        );
    }
}
