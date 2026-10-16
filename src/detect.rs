//! Native detection of already-installed Glowing Cat projects.
//!
//! The local `installed.json` only records installs performed *through this
//! app*. To notice software that was already present on first run — or
//! installed/updated/removed outside this app — we also ask the operating
//! system directly:
//!
//! - **Linux (Debian)**: `dpkg-query -W -f='${Version}' <pkg>`
//! - **Linux (RPM)**: `rpm -q --qf '%{VERSION}' <pkg>`
//! - **macOS**: read `CFBundleShortVersionString` from the installed
//!   `<App>.app/Contents/Info.plist` in /Applications or ~/Applications.
//! - **Windows**: scan the Uninstall registry hives for a matching
//!   `DisplayName` and read its `DisplayVersion`.
//!
//! Detection is best-effort: a `None` result means "not detected", never an
//! error that blocks the UI. Results are merged with the local state (native
//! detection takes precedence when present, since it reflects reality).

use crate::model::Project;
use crate::platform::{InstallScope, PackageType};

/// Detect the installed version of `project` for the given target package type
/// and install scope. Returns `None` if not installed / not detectable.
///
/// `recorded_path` is the exact install location this app previously recorded
/// (macOS: the `.app` bundle path). When present and consistent with the
/// selected scope, macOS detection trusts it, which makes detection robust to
/// bundle names that differ from the catalog's expected name.
///
/// Scope only changes macOS detection (which Applications folder to look in);
/// dpkg/rpm (Linux) and the registry (Windows) are queried the same way for
/// both scopes.
pub fn detect_installed_version(
    project: &Project,
    pkg: PackageType,
    scope: InstallScope,
    recorded_path: Option<&str>,
) -> Option<String> {
    match pkg {
        PackageType::Deb => detect_deb(&project.deb_rpm_package()),
        PackageType::Rpm => detect_rpm(&project.deb_rpm_package()),
        PackageType::Macos => detect_macos(&project.macos_app_name(), scope, recorded_path),
        PackageType::Windows => detect_windows(&project.windows_display_name()),
    }
}

/// Human-readable description of where this project is / would be installed for
/// the given package type and scope, used to guide manual removal.
pub fn install_location_hint(project: &Project, pkg: PackageType, scope: InstallScope) -> String {
    match pkg {
        PackageType::Deb => format!("dpkg package '{}'", project.deb_rpm_package()),
        PackageType::Rpm => format!("rpm package '{}'", project.deb_rpm_package()),
        PackageType::Macos => {
            let dir = match scope {
                InstallScope::AllUsers => "/Applications",
                InstallScope::ThisUser => "~/Applications",
            };
            format!("{dir}/{}.app", project.macos_app_name())
        }
        PackageType::Windows => {
            format!("Add/Remove Programs: '{}'", project.windows_display_name())
        }
    }
}

/// Normalise a raw native version (e.g. "1.2.0-1", "2:1.2.0") to the bare
/// upstream version so it compares cleanly against GitHub tags.
fn normalise_version(raw: &str) -> String {
    let s = raw.trim();
    // Strip a Debian epoch like "2:".
    let s = s.splitn(2, ':').last().unwrap_or(s);
    // Strip a Debian/RPM release suffix like "-1" or "-1.fc39".
    let s = s.split('-').next().unwrap_or(s);
    s.trim().to_string()
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

/// Query dpkg for a package version. Works regardless of target OS at compile
/// time (the command simply fails on non-Debian systems, yielding `None`).
fn detect_deb(pkg: &str) -> Option<String> {
    let out = std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Version}", pkg])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout);
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(normalise_version(v))
    }
}

/// Query rpm for a package version.
fn detect_rpm(pkg: &str) -> Option<String> {
    let out = std::process::Command::new("rpm")
        .args(["-q", "--qf", "%{VERSION}", pkg])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout);
    let v = v.trim();
    // rpm prints "package foo is not installed" to stdout on some configs.
    if v.is_empty() || v.contains("not installed") {
        None
    } else {
        Some(normalise_version(v))
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

/// Look for `<app>.app` in the Applications folder for the given scope and read
/// its short version string from Info.plist. All Users → `/Applications`; This
/// User → `~/Applications`.
///
/// If `recorded_path` points at a bundle that lives inside the selected scope's
/// directory and exists, it is used directly — covering apps whose real bundle
/// name differs from the catalog's expected name.
fn detect_macos(app_name: &str, scope: InstallScope, recorded_path: Option<&str>) -> Option<String> {
    let dir: std::path::PathBuf = match scope {
        InstallScope::AllUsers => std::path::PathBuf::from("/Applications"),
        InstallScope::ThisUser => directories::BaseDirs::new()?.home_dir().join("Applications"),
    };

    // Candidate bundle paths, most specific first: the exact recorded path (only
    // if it lives in the selected scope's dir), then the catalog-named bundle.
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(rp) = recorded_path {
        if !rp.is_empty() {
            let p = std::path::PathBuf::from(rp);
            if p.starts_with(&dir) {
                candidates.push(p);
            }
        }
    }
    candidates.push(dir.join(format!("{app_name}.app")));

    for app in candidates {
        let plist = app.join("Contents/Info.plist");
        if plist.exists() {
            if let Some(v) = read_plist_version(&plist) {
                return Some(normalise_version(&v));
            }
            // Bundle exists but version unreadable: still "installed".
            return Some(String::new());
        }
    }
    None
}

/// Read CFBundleShortVersionString from an Info.plist using the OS tools.
/// Tries `defaults read` first (handles binary + xml plists), then a crude
/// XML scan as a fallback.
fn read_plist_version(plist: &std::path::Path) -> Option<String> {
    // `defaults read <path-without-extension> KEY`
    let path_no_ext = plist.with_extension("");
    let out = std::process::Command::new("defaults")
        .arg("read")
        .arg(&path_no_ext)
        .arg("CFBundleShortVersionString")
        .output()
        .ok();
    if let Some(out) = out {
        if out.status.success() {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }

    // Fallback: read the file and scan for the key (works for XML plists).
    let text = std::fs::read_to_string(plist).ok()?;
    let key = "CFBundleShortVersionString";
    let idx = text.find(key)?;
    let after = &text[idx + key.len()..];
    let open = after.find("<string>")? + "<string>".len();
    let close = after[open..].find("</string>")?;
    Some(after[open..open + close].trim().to_string())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// Scan the Windows Uninstall registry hives for an entry whose DisplayName
/// contains `display_substr`, returning its DisplayVersion.
///
/// Implemented by shelling out to `reg query` so we need no extra crates.
#[cfg(target_os = "windows")]
fn detect_windows(display_substr: &str) -> Option<String> {
    const ROOTS: [&str; 3] = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    for root in ROOTS {
        // Enumerate subkeys.
        let list = std::process::Command::new("reg")
            .args(["query", root])
            .output()
            .ok()?;
        if !list.status.success() {
            continue;
        }
        let listing = String::from_utf8_lossy(&list.stdout);
        for subkey in listing.lines().map(|l| l.trim()).filter(|l| l.starts_with(root)) {
            // Read DisplayName for this subkey.
            let dn = reg_value(subkey, "DisplayName");
            if let Some(name) = dn {
                if name
                    .to_ascii_lowercase()
                    .contains(&display_substr.to_ascii_lowercase())
                {
                    let ver = reg_value(subkey, "DisplayVersion").unwrap_or_default();
                    return Some(normalise_version(&ver));
                }
            }
        }
    }
    None
}

/// Read a single registry string value via `reg query <key> /v <name>`.
#[cfg(target_os = "windows")]
fn reg_value(key: &str, name: &str) -> Option<String> {
    let out = std::process::Command::new("reg")
        .args(["query", key, "/v", name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Lines look like:  "    DisplayName    REG_SZ    Some App"
    for line in text.lines() {
        if let Some(pos) = line.find(name) {
            let rest = &line[pos + name.len()..];
            // Split on REG_SZ / REG_* and take the remainder.
            if let Some(tpos) = rest.find("REG_") {
                let after = &rest[tpos..];
                if let Some(sp) = after.find(char::is_whitespace) {
                    return Some(after[sp..].trim().to_string());
                }
            }
        }
    }
    None
}

/// Non-Windows builds: no registry, so always `None`.
#[cfg(not(target_os = "windows"))]
fn detect_windows(_display_substr: &str) -> Option<String> {
    None
}

/// Run the Windows uninstaller for the program whose DisplayName contains
/// `display_substr`, by reading its `UninstallString` (or `QuietUninstallString`)
/// from the Uninstall registry hives and executing it. Returns `true` if an
/// uninstaller was found and launched.
#[cfg(target_os = "windows")]
pub fn run_windows_uninstall(display_substr: &str) -> Result<bool, crate::install::InstallError> {
    const ROOTS: [&str; 3] = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    for root in ROOTS {
        let list = match std::process::Command::new("reg").args(["query", root]).output() {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };
        let listing = String::from_utf8_lossy(&list.stdout);
        for subkey in listing.lines().map(|l| l.trim()).filter(|l| l.starts_with(root)) {
            let Some(name) = reg_value(subkey, "DisplayName") else {
                continue;
            };
            if !name
                .to_ascii_lowercase()
                .contains(&display_substr.to_ascii_lowercase())
            {
                continue;
            }
            // Prefer a quiet uninstall string when the program provides one.
            let cmd = reg_value(subkey, "QuietUninstallString")
                .or_else(|| reg_value(subkey, "UninstallString"));
            let Some(cmd) = cmd else {
                return Ok(false);
            };
            // Run the command line through cmd so quoting/args are honoured.
            std::process::Command::new("cmd")
                .args(["/C", &cmd])
                .spawn()
                .map_err(|e| crate::install::InstallError::Launch(format!("uninstall: {e}")))?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Non-Windows builds: nothing to run.
#[cfg(not(target_os = "windows"))]
pub fn run_windows_uninstall(_display_substr: &str) -> Result<bool, crate::install::InstallError> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_strips_epoch_and_release() {
        assert_eq!(normalise_version("1.2.0"), "1.2.0");
        assert_eq!(normalise_version("1.2.0-1"), "1.2.0");
        assert_eq!(normalise_version("2:1.2.0-1.fc39"), "1.2.0");
        assert_eq!(normalise_version("  2.8.0  "), "2.8.0");
    }

    #[test]
    fn missing_package_is_none() {
        // A package that certainly does not exist should yield None, not panic,
        // on any OS (the command either errors or reports not-installed).
        assert!(detect_deb("definitely-not-a-real-package-xyz").is_none());
        assert!(detect_rpm("definitely-not-a-real-package-xyz").is_none());
    }
}
