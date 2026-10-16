//! Native macOS menu bar (built with [`muda`]).
//!
//! On macOS the menu bar lives in the system bar at the top of the screen. Slint's
//! declarative `MenuBar` *does* render natively via muda, but in Slint 1.18 it
//! always synthesises its own app-named menu and gives no way to route its
//! About item back to our callback (menu "roles" are not implemented yet). So
//! on macOS we bypass the Slint `MenuBar` entirely and build the menu ourselves
//! with muda, which lets us put *About Glowing Cat Installer* in the real
//! Application menu (the bold, app-named one) exactly where macOS users expect
//! it — rather than creating a second, duplicate menu.
//!
//! The muda version is pinned in `Cargo.toml` to the same one Slint's winit
//! backend uses, so there is a single `muda` in the dependency tree and its
//! process-global `MenuEvent` channel is shared. Because our UI contains no
//! Slint `MenuBar` on macOS (`use-slint-menubar` is left false and the backend
//! is created with the default menu bar disabled), Slint never installs its own
//! muda event handler, leaving `MenuEvent::receiver()` free for us to poll.

use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use slint::ComponentHandle;

use crate::MainWindow;

/// Stable id for our custom About item so we can route activation back to the UI.
const ID_ABOUT: &str = "app.about";

/// Build the native macOS menu, install it as the application menu, and start
/// polling for menu events (routing About to the window's `open-about`).
///
/// The returned [`Menu`] must be kept alive for the lifetime of the app, so the
/// caller should hold on to it (dropping it would tear the menu down).
pub fn install(window: &MainWindow) -> Menu {
    let menu = Menu::new();

    // ---- Application menu (the bold, app-named menu) --------------------
    // Order follows the macOS HIG: About, then the standard Services/Hide/Quit
    // block built from predefined items. Our About replaces the predefined one
    // so it opens our themed dialog.
    let app_menu = Submenu::new("Glowing Cat Installer", true);
    let about_item = MenuItem::with_id(ID_ABOUT, "About Glowing Cat Installer", true, None);
    menu.append(&app_menu).unwrap();
    app_menu
        .append_items(&[
            &about_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])
        .unwrap();

    // ---- File -----------------------------------------------------------
    // No document model; a Close Window keeps the menu non-empty and behaves
    // natively (Cmd-W).
    let file_menu = Submenu::new("File", true);
    menu.append(&file_menu).unwrap();
    file_menu
        .append_items(&[&PredefinedMenuItem::close_window(None)])
        .unwrap();

    // ---- Edit -----------------------------------------------------------
    // Standard predefined edit items so the usual accelerators work.
    let edit_menu = Submenu::new("Edit", true);
    menu.append(&edit_menu).unwrap();
    edit_menu
        .append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ])
        .unwrap();

    // Install into the shared NSApp. Must run on the main thread (we are).
    menu.init_for_nsapp();

    install_event_handler(window);

    menu
}

/// Receive muda menu events and dispatch them to the window on the UI thread.
///
/// We register our own handler via [`MenuEvent::set_event_handler`]. The handler
/// runs on the main thread; we hop onto the Slint event loop with
/// [`slint::invoke_from_event_loop`] before touching the window. Because we
/// disable Slint's default menu bar (see `main`), Slint never installs its own
/// handler, so ours is the one muda uses.
fn install_event_handler(window: &MainWindow) {
    let weak = window.as_weak();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                dispatch(&window, &event.id);
            }
        });
    }));
}

/// Route a menu id to the corresponding window callback. Unknown ids (e.g. the
/// predefined Quit/Edit items, which macOS handles itself) are ignored.
fn dispatch(window: &MainWindow, id: &MenuId) {
    if id.as_ref() == ID_ABOUT {
        window.invoke_open_about();
    }
}
