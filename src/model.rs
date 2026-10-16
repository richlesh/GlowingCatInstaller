//! The static catalog of Glowing Cat Projects available for installation.
//!
//! Each entry maps a human-facing name and description to the GitHub repo under
//! `github.com/richlesh` and a stubbed icon path. Icons live in
//! `resources/icons/<icon>` and are supplied by the project owner; a missing
//! icon falls back to a placeholder at runtime.

/// The GitHub account that owns every Glowing Cat project.
pub const GITHUB_OWNER: &str = "richlesh";

/// A single installable project.
#[derive(Debug, Clone)]
pub struct Project {
    /// Display name shown under the icon.
    pub name: &'static str,
    /// Repository name under `github.com/richlesh/<repo>`.
    pub repo: &'static str,
    /// Short, one-line description shown under the name.
    pub description: &'static str,
    /// Icon filename, relative to `resources/icons/`.
    pub icon: &'static str,
    /// Debian/RPM package name used to query dpkg/rpm. When `None`, a default
    /// is derived from the repo name (lowercased). Some projects use a `-gui`
    /// suffix or an abbreviated name in their packages.
    pub pkg_name: Option<&'static str>,
    /// macOS application bundle name (without the `.app` extension) as it
    /// appears in /Applications. When `None`, defaults to the display name.
    pub macos_app: Option<&'static str>,
    /// Substring to match against Windows "DisplayName" in the Uninstall
    /// registry. When `None`, defaults to the display name.
    pub win_display: Option<&'static str>,
}

impl Project {
    /// Full `owner/repo` slug used by the GitHub API.
    pub fn slug(&self) -> String {
        format!("{GITHUB_OWNER}/{}", self.repo)
    }

    /// Public releases page, used for the download-then-open fallback and the
    /// "view on GitHub" affordance.
    #[allow(dead_code)]
    pub fn releases_url(&self) -> String {
        format!("https://github.com/{GITHUB_OWNER}/{}/releases", self.repo)
    }

    /// Debian/RPM package name to query, with a repo-derived default.
    pub fn deb_rpm_package(&self) -> String {
        self.pkg_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.repo.to_ascii_lowercase())
    }

    /// macOS .app bundle name (no extension), defaulting to the display name.
    pub fn macos_app_name(&self) -> String {
        self.macos_app.unwrap_or(self.name).to_string()
    }

    /// Windows DisplayName substring to match, defaulting to the display name.
    pub fn windows_display_name(&self) -> String {
        self.win_display.unwrap_or(self.name).to_string()
    }
}

/// The full catalog. Order here is the order shown in the grid.
///
/// Descriptions are sourced from the repos' GitHub descriptions where present,
/// and summarised otherwise.
pub fn catalog() -> Vec<Project> {
    vec![
        Project {
            name: "Zubrenn",
            repo: "Zubrenn",
            description: "World colonization 4x game.",
            icon: "Zubrenn.png",
            pkg_name: None, // zubrenn
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "SpaceServal",
            repo: "SpaceServal",
            description: "A simple disk space usage analyzer.",
            icon: "SpaceServal.png",
            pkg_name: Some("spaceserval-gui"), // deb: spaceserval-gui_1.1.0-1_amd64.deb
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "ArchiveSphynx",
            repo: "ArchiveSphynx",
            description: "Archive management utility.",
            icon: "ArchiveSphynx.png",
            pkg_name: None, // archivesphynx
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "TagTiger",
            repo: "TagTiger",
            description: "Audio/file tag editor.",
            icon: "TagTiger.png",
            pkg_name: Some("tagtiger-gui"), // deb: tagtiger-gui_1.2.0-1_amd64.deb
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "NeuroPanther Chat",
            repo: "NeuroPanther-Chat",
            description: "Chat client for multiple LLM providers.",
            icon: "NeuroPanther-Chat.png",
            pkg_name: Some("neuropanther-chat"),
            // The installed .app bundle is "NeuroPanther Chat.app" (a space, the
            // display name), not the hyphenated repo name.
            macos_app: Some("NeuroPanther Chat"),
            win_display: Some("NeuroPanther"),
        },
        Project {
            name: "PurplePlatypus",
            repo: "PurplePlatypus",
            description: "Easy to use Markdown editor.",
            icon: "PurplePlatypus.png",
            pkg_name: None, // purpleplatypus
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "BudgetLion",
            repo: "BudgetLion",
            description: "Personal budgeting application.",
            icon: "BudgetLion.png",
            pkg_name: None, // budgetlion
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "TransferJaguar",
            repo: "TransferJaguar",
            description: "SFTP file manager.",
            icon: "TransferJaguar.png",
            pkg_name: None, // transferjaguar
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "RenameCheetah",
            repo: "RenameCheetah",
            description: "Bulk file renaming tool.",
            icon: "RenameCheetah.png",
            pkg_name: None, // renamecheetah
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "BengalCalc",
            repo: "BengalCalc",
            description: "Calculator application.",
            icon: "BengalCalc.png",
            pkg_name: None, // bengalcalc
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "LayoutLynx",
            repo: "LayoutLynx",
            description: "LayoutLynx CSS editor.",
            icon: "LayoutLynx.png",
            pkg_name: None, // layoutlynx
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "Joshua",
            repo: "Joshua",
            description: "WarGames inspired retro-games.",
            icon: "Joshua.png",
            pkg_name: None, // joshua
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "ASDQChart",
            repo: "ASDQChart",
            description: "ASDQ Charting application.",
            icon: "ASDQChart.png",
            pkg_name: None, // asdqchart
            macos_app: None,
            win_display: None,
        },
        Project {
            name: "CPUSim64 V2",
            repo: "CPUSim64V2",
            description: "CPU Simulator, 64-bit, version 2.",
            icon: "CPUSim64.png",
            pkg_name: Some("cpusim64"), // deb: cpusim64_2.8.0_amd64.deb
            macos_app: Some("CPUSim64"), // dmg: CPUSim64-...dmg
            win_display: Some("CPUSim64"),
        },
        Project {
            name: "CodegenIDE",
            repo: "CodegenIDE",
            description: "Code generation IDE.",
            icon: "CodegenIDE.png",
            pkg_name: None, // codegenide
            macos_app: None,
            win_display: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_populated() {
        let c = catalog();
        assert_eq!(c.len(), 15);
        // No empty fields.
        for p in &c {
            assert!(!p.name.is_empty());
            assert!(!p.repo.is_empty());
            assert!(!p.description.is_empty());
            assert!(p.icon.ends_with(".png"));
        }
    }

    #[test]
    fn slugs_are_well_formed() {
        for p in catalog() {
            assert_eq!(p.slug(), format!("richlesh/{}", p.repo));
        }
    }

    #[test]
    fn detection_identifiers() {
        let c = catalog();
        let by_repo = |r: &str| c.iter().find(|p| p.repo == r).unwrap();

        // Defaults derive from the repo / display name.
        let zub = by_repo("Zubrenn");
        assert_eq!(zub.deb_rpm_package(), "zubrenn");
        assert_eq!(zub.macos_app_name(), "Zubrenn");
        assert_eq!(zub.windows_display_name(), "Zubrenn");

        // Known overrides.
        assert_eq!(by_repo("SpaceServal").deb_rpm_package(), "spaceserval-gui");
        assert_eq!(by_repo("TagTiger").deb_rpm_package(), "tagtiger-gui");

        let cpu = by_repo("CPUSim64V2");
        assert_eq!(cpu.deb_rpm_package(), "cpusim64");
        assert_eq!(cpu.macos_app_name(), "CPUSim64");
        assert_eq!(cpu.windows_display_name(), "CPUSim64");

        // NeuroPanther's installed bundle uses a space (the display name).
        assert_eq!(by_repo("NeuroPanther-Chat").macos_app_name(), "NeuroPanther Chat");
    }
}
