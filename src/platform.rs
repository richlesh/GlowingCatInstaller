//! Host platform detection and installer package-type modelling.
//!
//! The architecture is always derived from the running machine. The target OS /
//! package type defaults to the host but can be overridden by the user via the
//! OS popup in the UI (useful for inspecting what would be downloaded on another
//! platform, and for building portable logic).

use std::fmt;

/// CPU architecture of the installer we want to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

impl Arch {
    /// Detect the architecture of the current machine.
    pub fn host() -> Arch {
        // `std::env::consts::ARCH` is resolved at compile time for the target
        // the binary was built for, which is exactly what we want: the binary
        // runs on the arch it was compiled for.
        match std::env::consts::ARCH {
            "x86_64" => Arch::X64,
            "aarch64" | "arm" => Arch::Arm64,
            // Fall back to x64 for anything exotic; most releases ship x64.
            _ => Arch::X64,
        }
    }

    /// Human-readable label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
        }
    }

    /// Tokens that, when present in an asset filename, indicate this arch.
    /// Ordered by specificity. Used by the asset matcher.
    pub fn tokens(self) -> &'static [&'static str] {
        match self {
            Arch::X64 => &["x86_64", "amd64", "x64", "win64", "64bit", "x86-64"],
            Arch::Arm64 => &["aarch64", "arm64", "aarch-64"],
        }
    }

    /// Tokens that indicate the *other* architecture (used to reject
    /// mismatches, e.g. so "arm64" does not accidentally match an x64 request).
    pub fn opposing_tokens(self) -> &'static [&'static str] {
        match self {
            Arch::X64 => Arch::Arm64.tokens(),
            Arch::Arm64 => Arch::X64.tokens(),
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Installer package format / target OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageType {
    Deb,
    Rpm,
    Macos,
    Windows,
}

impl PackageType {
    /// All variants, in the order they should appear in the OS popup.
    pub fn all() -> [PackageType; 4] {
        [
            PackageType::Deb,
            PackageType::Rpm,
            PackageType::Macos,
            PackageType::Windows,
        ]
    }

    /// Best default package type for the host OS.
    pub fn host_default() -> PackageType {
        match std::env::consts::OS {
            "macos" => PackageType::Macos,
            "windows" => PackageType::Windows,
            // On Linux we can't perfectly know deb vs rpm; probe for common
            // package managers and fall back to deb.
            "linux" => detect_linux_package_type(),
            _ => PackageType::Deb,
        }
    }

    /// Label shown in the OS popup.
    pub fn label(self) -> &'static str {
        match self {
            PackageType::Deb => "Linux (Debian)",
            PackageType::Rpm => "Linux (RPM)",
            PackageType::Macos => "macOS",
            PackageType::Windows => "Windows",
        }
    }

    /// Map a popup index back to a package type.
    pub fn from_index(i: usize) -> PackageType {
        PackageType::all()
            .get(i)
            .copied()
            .unwrap_or(PackageType::Deb)
    }

    /// Index of this package type within `all()`.
    pub fn index(self) -> usize {
        PackageType::all()
            .iter()
            .position(|p| *p == self)
            .unwrap_or(0)
    }

    /// File extensions that are valid for this package type, ordered by
    /// preference. (Windows prefers .exe setup installers over .msi, but both
    /// are acceptable.)
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            PackageType::Deb => &["deb"],
            PackageType::Rpm => &["rpm"],
            PackageType::Macos => &["dmg", "pkg"],
            PackageType::Windows => &["exe", "msi"],
        }
    }
}

impl fmt::Display for PackageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Who an install is for: the whole machine (system-wide) or just the current
/// user. Controls the install location and whether privilege elevation is
/// needed.
///
/// Platform notes:
/// - **macOS**: `AllUsers` -> `/Applications` (admin prompt); `ThisUser` ->
///   `~/Applications` (no prompt).
/// - **Windows (.msi)**: `AllUsers` -> per-machine (`ALLUSERS=1`, UAC prompt);
///   `ThisUser` -> per-user (`MSIINSTALLPERUSER=1`, no elevation).
/// - **Linux (.deb/.rpm)**: apt/dnf are system-wide only, so the install is
///   always effectively `AllUsers` regardless of this setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    AllUsers,
    ThisUser,
}

impl InstallScope {
    /// All variants, in the order they appear in the "Install for:" popup.
    /// `ThisUser` is first so it is the default selection.
    pub fn all() -> [InstallScope; 2] {
        [InstallScope::ThisUser, InstallScope::AllUsers]
    }

    /// Label shown in the "Install for:" popup.
    pub fn label(self) -> &'static str {
        match self {
            InstallScope::AllUsers => "All Users",
            InstallScope::ThisUser => "This User",
        }
    }

    /// Map a popup index back to a scope.
    pub fn from_index(i: usize) -> InstallScope {
        InstallScope::all().get(i).copied().unwrap_or(InstallScope::ThisUser)
    }

    /// Index of this scope within `all()`.
    pub fn index(self) -> usize {
        InstallScope::all()
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0)
    }
}

impl fmt::Display for InstallScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Probe the Linux host for its native package format.
#[cfg(target_os = "linux")]
fn detect_linux_package_type() -> PackageType {
    use std::path::Path;
    // Presence of dpkg -> Debian family; rpm -> RPM family.
    if Path::new("/usr/bin/dpkg").exists() || Path::new("/etc/debian_version").exists() {
        PackageType::Deb
    } else if Path::new("/usr/bin/rpm").exists() || Path::new("/etc/redhat-release").exists() {
        PackageType::Rpm
    } else {
        PackageType::Deb
    }
}

#[cfg(not(target_os = "linux"))]
fn detect_linux_package_type() -> PackageType {
    PackageType::Deb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_index_roundtrip() {
        for p in PackageType::all() {
            assert_eq!(PackageType::from_index(p.index()), p);
        }
    }

    #[test]
    fn scope_index_roundtrip_and_default() {
        for s in InstallScope::all() {
            assert_eq!(InstallScope::from_index(s.index()), s);
        }
        // Index 0 is the default (This User).
        assert_eq!(InstallScope::from_index(0), InstallScope::ThisUser);
        // Out-of-range falls back to This User.
        assert_eq!(InstallScope::from_index(99), InstallScope::ThisUser);
    }

    #[test]
    fn arch_tokens_disjoint() {
        for t in Arch::X64.tokens() {
            assert!(!Arch::Arm64.tokens().contains(t), "token {t} in both arches");
        }
    }
}
