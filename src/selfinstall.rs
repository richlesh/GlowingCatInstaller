//! Optional self-installer: lets the running app install *itself* onto the
//! current machine into the standard per-OS location, register a launcher /
//! shortcut, and later uninstall itself.
//!
//! Scope by platform:
//!
//! - macOS:   `/Applications/Glowing Cat Software Installer.app` (system-wide;
//!   prompts for administrator privileges to write to /Applications).
//! - Linux:   `~/.local/bin/glowing-cat-installer` + a `.desktop` entry in
//!   `~/.local/share/applications/` + icons under
//!   `~/.local/share/glowing-cat-installer/` (user-scoped).
//! - Windows: `%LOCALAPPDATA%\Programs\GlowingCatInstaller\` + a Start Menu
//!   shortcut (.lnk) created via a PowerShell/WScript.Shell call (user-scoped).
//!
//! The feature is entirely optional — the app runs fine un-installed. Status
//! detection lets the UI show "Install this app" vs "Uninstall this app".

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use directories::BaseDirs;

/// Human display name of the app.
pub const APP_DISPLAY_NAME: &str = "Glowing Cat Software Installer";
/// Binary / short name.
pub const APP_BIN_NAME: &str = "glowing-cat-installer";

/// Errors from self-install operations.
#[derive(Debug)]
#[allow(dead_code)] // `Platform` is only constructed on some targets (Windows / unsupported).
pub enum SelfInstallError {
    Io(String),
    NoLocation(String),
    Platform(String),
}

impl std::fmt::Display for SelfInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelfInstallError::Io(e) => write!(f, "file error: {e}"),
            SelfInstallError::NoLocation(e) => write!(f, "could not resolve install location: {e}"),
            SelfInstallError::Platform(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SelfInstallError {}

impl From<std::io::Error> for SelfInstallError {
    fn from(e: std::io::Error) -> Self {
        SelfInstallError::Io(e.to_string())
    }
}

/// Whether the app currently appears to be installed in its standard location.
pub fn is_installed() -> bool {
    installed_marker_path().map(|p| p.exists()).unwrap_or(false)
}

/// The path that, if present, indicates a completed self-install. This is the
/// installed executable (macOS: the binary inside the bundle).
fn installed_marker_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(macos_app_dir()?.join("Contents/MacOS").join(APP_BIN_NAME))
    }
    #[cfg(target_os = "linux")]
    {
        Some(linux_bin_path()?)
    }
    #[cfg(target_os = "windows")]
    {
        Some(windows_install_dir()?.join(format!("{APP_BIN_NAME}.exe")))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Human-readable description of where install would place the app (for the UI).
pub fn install_location_hint() -> String {
    installed_marker_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unsupported platform".to_string())
}

/// Path of the currently-running executable.
fn current_exe() -> Result<PathBuf, SelfInstallError> {
    std::env::current_exe().map_err(|e| SelfInstallError::Io(e.to_string()))
}

/// Copy a file, creating parent directories, and preserve the executable bit on
/// unix.
fn copy_executable(src: &Path, dst: &Path) -> Result<(), SelfInstallError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dst)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dst, perms)?;
    }
    Ok(())
}

/// Try to find the app's icons directory next to the running binary so we can
/// copy icons during install (best-effort; install still works without them).
fn source_icons_dir() -> Option<PathBuf> {
    // When run via `cargo run`, resources/ is in the working dir.
    let cwd = std::env::current_dir().ok()?.join("resources/icons");
    if cwd.exists() {
        return Some(cwd);
    }
    // When run from inside a macOS .app bundle, icons live in Resources.
    if let Ok(exe) = current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("../Resources/resources/icons");
            if bundled.exists() {
                return Some(bundled);
            }
        }
    }
    None
}

/// Locate the macOS .icns icon next to the running binary / in the working dir.
#[cfg(target_os = "macos")]
fn source_icns_path() -> Option<PathBuf> {
    // Running via `cargo run`: resources/icons/app_icon.icns in the cwd.
    let cwd = std::env::current_dir().ok()?.join("resources/icons/app_icon.icns");
    if cwd.exists() {
        return Some(cwd);
    }
    // Running from an installed .app: the icns lives in Contents/Resources.
    if let Ok(exe) = current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("../Resources/app_icon.icns");
            if bundled.exists() {
                return Some(bundled);
            }
        }
    }
    None
}
#[cfg(target_os = "macos")]
fn macos_app_dir() -> Option<PathBuf> {
    // System-wide location so the installer is available to all users.
    Some(PathBuf::from("/Applications").join(format!("{APP_DISPLAY_NAME}.app")))
}

/// Run a shell command with macOS administrator privileges via an osascript
/// prompt. The user sees a native password dialog; cancelling yields an error.
#[cfg(target_os = "macos")]
fn run_with_admin(shell_cmd: &str) -> Result<(), SelfInstallError> {
    let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{escaped}\" with administrator privileges");
    let status = std::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
        .map_err(|e| SelfInstallError::Platform(format!("osascript: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(SelfInstallError::Platform(
            "administrator authorization was cancelled or failed".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
pub fn install() -> Result<String, SelfInstallError> {
    let app_dir = macos_app_dir()
        .ok_or_else(|| SelfInstallError::NoLocation("/Applications".into()))?;

    // Build the bundle in a user-writable temp staging dir first (no admin
    // needed), then move it into /Applications in one elevated step.
    let staging_root = std::env::temp_dir().join(format!("gci-selfinstall-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging_root);
    let staging = staging_root.join(format!("{APP_DISPLAY_NAME}.app"));
    let macos_dir = staging.join("Contents/MacOS");
    let resources_dir = staging.join("Contents/Resources");
    fs::create_dir_all(&macos_dir)?;
    fs::create_dir_all(&resources_dir)?;

    // Copy the binary.
    let dst_bin = macos_dir.join(APP_BIN_NAME);
    copy_executable(&current_exe()?, &dst_bin)?;

    // Application icon (.icns), if available next to the running app.
    let icon_line = if let Some(icns) = source_icns_path() {
        let _ = fs::copy(&icns, resources_dir.join("app_icon.icns"));
        "    <key>CFBundleIconFile</key><string>app_icon</string>\n"
    } else {
        ""
    };

    // Minimal Info.plist.
    let version = env!("CARGO_PKG_VERSION");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>{APP_DISPLAY_NAME}</string>
    <key>CFBundleExecutable</key><string>{APP_BIN_NAME}</string>
    <key>CFBundleIdentifier</key><string>cloud.lesh.glowingcatinstaller</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
{icon_line}    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    );
    fs::write(staging.join("Contents/Info.plist"), plist)?;

    // Copy icons so the installed app can display them.
    if let Some(src_icons) = source_icons_dir() {
        let dst_icons = resources_dir.join("resources/icons");
        let _ = copy_dir(&src_icons, &dst_icons);
    }

    // Move the staged bundle into /Applications with one admin prompt
    // (replacing any existing copy, e.g. on upgrade).
    let src = staging.to_string_lossy().replace('\'', "'\\''");
    let dst = app_dir.to_string_lossy().replace('\'', "'\\''");
    run_with_admin(&format!("rm -rf '{dst}' && /bin/mv '{src}' '{dst}'"))?;
    let _ = fs::remove_dir_all(&staging_root);

    Ok(app_dir.display().to_string())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<(), SelfInstallError> {
    if let Some(app_dir) = macos_app_dir() {
        if app_dir.exists() {
            // /Applications is root-owned, so elevate the removal.
            let dst = app_dir.to_string_lossy().replace('\'', "'\\''");
            run_with_admin(&format!("rm -rf '{dst}'"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
fn linux_bin_path() -> Option<PathBuf> {
    let home = BaseDirs::new()?.home_dir().to_path_buf();
    Some(home.join(".local/bin").join(APP_BIN_NAME))
}

#[cfg(target_os = "linux")]
fn linux_desktop_path() -> Option<PathBuf> {
    let home = BaseDirs::new()?.home_dir().to_path_buf();
    Some(
        home.join(".local/share/applications")
            .join(format!("{APP_BIN_NAME}.desktop")),
    )
}

#[cfg(target_os = "linux")]
pub fn install() -> Result<String, SelfInstallError> {
    let bin = linux_bin_path()
        .ok_or_else(|| SelfInstallError::NoLocation("home directory".into()))?;
    copy_executable(&current_exe()?, &bin)?;

    // Copy icons under ~/.local/share/glowing-cat-installer/icons.
    let home = BaseDirs::new()
        .ok_or_else(|| SelfInstallError::NoLocation("home directory".into()))?
        .home_dir()
        .to_path_buf();
    let icons_dst = home
        .join(".local/share")
        .join(APP_BIN_NAME)
        .join("icons");
    if let Some(src_icons) = source_icons_dir() {
        let _ = copy_dir(&src_icons, &icons_dst);
    }

    // Write a .desktop entry pointing at the installed binary.
    let desktop = linux_desktop_path()
        .ok_or_else(|| SelfInstallError::NoLocation("applications dir".into()))?;
    if let Some(parent) = desktop.parent() {
        fs::create_dir_all(parent)?;
    }
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={APP_DISPLAY_NAME}\n\
         Comment=Download and install the Glowing Cat Projects\n\
         Exec={}\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=Utility;System;\n",
        bin.display(),
        icons_dst.join("app_icon.png").display(),
    );
    fs::write(&desktop, entry)?;

    Ok(bin.display().to_string())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<(), SelfInstallError> {
    if let Some(bin) = linux_bin_path() {
        if bin.exists() {
            fs::remove_file(&bin)?;
        }
    }
    if let Some(desktop) = linux_desktop_path() {
        if desktop.exists() {
            fs::remove_file(&desktop)?;
        }
    }
    if let Some(home) = BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
        let icons = home.join(".local/share").join(APP_BIN_NAME);
        if icons.exists() {
            let _ = fs::remove_dir_all(&icons);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
fn windows_install_dir() -> Option<PathBuf> {
    // %LOCALAPPDATA%\Programs\GlowingCatInstaller
    let base = BaseDirs::new()?;
    Some(
        base.data_local_dir()
            .join("Programs")
            .join("GlowingCatInstaller"),
    )
}

#[cfg(target_os = "windows")]
fn windows_start_menu_lnk() -> Option<PathBuf> {
    // %APPDATA%\Microsoft\Windows\Start Menu\Programs\Glowing Cat Software Installer.lnk
    let base = BaseDirs::new()?;
    Some(
        base.config_dir() // %APPDATA%
            .join("Microsoft/Windows/Start Menu/Programs")
            .join(format!("{APP_DISPLAY_NAME}.lnk")),
    )
}

#[cfg(target_os = "windows")]
pub fn install() -> Result<String, SelfInstallError> {
    let dir = windows_install_dir()
        .ok_or_else(|| SelfInstallError::NoLocation("LOCALAPPDATA".into()))?;
    let dst_exe = dir.join(format!("{APP_BIN_NAME}.exe"));
    copy_executable(&current_exe()?, &dst_exe)?;

    // Copy icons next to the exe.
    if let Some(src_icons) = source_icons_dir() {
        let _ = copy_dir(&src_icons, &dir.join("resources/icons"));
    }

    // Create a Start Menu shortcut using WScript.Shell via PowerShell.
    if let Some(lnk) = windows_start_menu_lnk() {
        if let Some(parent) = lnk.parent() {
            fs::create_dir_all(parent)?;
        }
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk}');\
             $s.TargetPath='{target}';\
             $s.WorkingDirectory='{wd}';\
             $s.Save()",
            lnk = lnk.display(),
            target = dst_exe.display(),
            wd = dir.display(),
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .map_err(|e| SelfInstallError::Platform(format!("shortcut: {e}")))?;
        if !status.success() {
            return Err(SelfInstallError::Platform(
                "failed to create Start Menu shortcut".into(),
            ));
        }
    }

    Ok(dst_exe.display().to_string())
}

#[cfg(target_os = "windows")]
pub fn uninstall() -> Result<(), SelfInstallError> {
    if let Some(dir) = windows_install_dir() {
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
    }
    if let Some(lnk) = windows_start_menu_lnk() {
        if lnk.exists() {
            let _ = fs::remove_file(&lnk);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fallback for unsupported targets (keeps the crate compiling everywhere).
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn install() -> Result<String, SelfInstallError> {
    Err(SelfInstallError::Platform("self-install unsupported on this OS".into()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn uninstall() -> Result<(), SelfInstallError> {
    Err(SelfInstallError::Platform("self-install unsupported on this OS".into()))
}

/// Recursively copy a directory (best-effort; used for icons).
#[allow(dead_code)]
fn copy_dir(src: &Path, dst: &Path) -> Result<(), SelfInstallError> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_hint_is_nonempty() {
        // On supported OSes we get a path; otherwise the "unsupported" string.
        assert!(!install_location_hint().is_empty());
    }

    #[test]
    fn copy_dir_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("gci-selftest-{}", std::process::id()));
        let src = tmp.join("src");
        let dst = tmp.join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), b"hello").unwrap();
        copy_dir(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        let _ = fs::remove_dir_all(&tmp);
    }
}
