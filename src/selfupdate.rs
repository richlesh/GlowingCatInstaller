//! Self-update: check this installer's *own* GitHub repo for a newer release
//! and install it through the same signed per-OS installer flow the catalog
//! projects use (download the matching `.dmg`/`.deb`/`.rpm`/`Setup.exe` and run
//! it). See `install::install_file`.
//!
//! The check is throttled by reusing the hourly `release_cache` in
//! [`InstallState`] (keyed by [`SELF_REPO`]), so it costs at most one
//! GitHub request per hour — the same budget discipline as the project checks.
//!
//! Why run the signed installer rather than swapping the binary in place: on
//! macOS an in-place binary swap breaks the bundle's code signature /
//! notarization, and on Windows a running `.exe` can't be overwritten. Handing
//! the freshly-downloaded, signed installer to the native install path avoids
//! both problems; the user relaunches to pick up the new version.

use crate::github;
use crate::install::{self, InstallState};
use crate::platform::{Arch, InstallScope, PackageType};

/// The installer's own GitHub repository (owner/repo).
pub const SELF_REPO: &str = "richlesh/GlowingCatInstaller";

/// macOS `.app` bundle name used when the self-update installs on macOS.
const SELF_APP_NAME: &str = "Glowing Cat Software Installer";
/// dpkg/rpm package name for the installer itself.
const SELF_PKG_NAME: &str = "glowing-cat-installer";

/// The version this running binary was built as.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A newer release of the installer that is available to download and run.
#[derive(Debug, Clone)]
pub struct SelfUpdate {
    /// The newer release tag (e.g. "v0.2.0").
    pub version: String,
    /// The best-matching installer asset filename for this OS/arch.
    pub asset_name: String,
    /// The asset's download URL.
    pub asset_url: String,
}

/// Convert cached assets into the `(name, url)` pairs the matcher expects.
fn pairs(assets: &[install::CachedAsset]) -> Vec<(String, String)> {
    assets
        .iter()
        .map(|a| (a.name.clone(), a.url.clone()))
        .collect()
}

/// Check for a newer release of the installer itself.
///
/// Uses the hourly cache in `state` (keyed by [`SELF_REPO`]); on a cache miss or
/// when `force` is set it hits GitHub once and persists the result. Returns
/// `Some(SelfUpdate)` only when the latest release is strictly newer than the
/// running version *and* a matching installer asset exists for this OS/arch.
///
/// `None` covers every "nothing to do / can't tell" case: up to date, no
/// release, no matching asset, network error, or rate limited — the caller
/// simply doesn't surface an update in those cases.
pub fn check(state: &mut InstallState, pkg: PackageType, arch: Arch, force: bool) -> Option<SelfUpdate> {
    // Release info: fresh cache, else fetch once and persist (mirrors the
    // project-check path in main.rs so both share the hourly budget).
    let (tag, assets): (Option<String>, Vec<(String, String)>) = {
        let fresh = if force {
            None
        } else {
            state.fresh_cache(SELF_REPO)
        };
        if let Some(c) = fresh {
            (c.tag_name.clone(), pairs(&c.assets))
        } else {
            match github::latest_release(SELF_REPO) {
                Ok(rel) => {
                    let cached_assets: Vec<install::CachedAsset> = rel
                        .assets
                        .iter()
                        .map(|a| install::CachedAsset {
                            name: a.name.clone(),
                            url: a.download_url.clone(),
                        })
                        .collect();
                    state.put_cache(SELF_REPO, Some(rel.tag_name.clone()), cached_assets.clone(), false);
                    let _ = install::save_state(state);
                    (Some(rel.tag_name.clone()), pairs(&cached_assets))
                }
                Err(github::FetchError::NoRelease) => {
                    state.put_cache(SELF_REPO, None, Vec::new(), true);
                    let _ = install::save_state(state);
                    (None, Vec::new())
                }
                // Network error / rate limited: don't cache, report nothing.
                Err(_) => return None,
            }
        }
    };

    let tag = tag?;
    // Only an update if the release is strictly newer than what we're running.
    if !github::is_newer(current_version(), &tag) {
        return None;
    }
    // And only if there's an installer asset for this OS/arch.
    let (name, url) = github::best_named_asset(&assets, pkg, arch)?;
    Some(SelfUpdate {
        version: tag,
        asset_name: name.clone(),
        asset_url: url.clone(),
    })
}

/// Download the update's installer asset and run it through the native install
/// path (the same `install_file` the catalog projects use). On success the new
/// version is installed/launched; the caller should tell the user to relaunch.
///
/// `scope` controls All Users vs. This User where the OS distinguishes them
/// (macOS, Windows .msi); it is ignored on Linux (apt/dnf are system-wide).
pub fn apply(update: &SelfUpdate, pkg: PackageType, scope: InstallScope) -> Result<install::InstallOutcome, install::InstallError> {
    let path = install::download_asset(&update.asset_url, &update.asset_name)?;
    install::install_file(&path, pkg, scope, SELF_APP_NAME, SELF_PKG_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_repo_is_the_installer() {
        assert_eq!(SELF_REPO, "richlesh/GlowingCatInstaller");
    }

    #[test]
    fn current_version_matches_cargo() {
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn no_update_when_cache_is_same_version() {
        // Seed the cache with exactly the running version and a matching macOS
        // asset; check() must report no update (not strictly newer).
        let mut state = InstallState::default();
        let same = format!("v{}", current_version());
        state.put_cache(
            SELF_REPO,
            Some(same),
            vec![install::CachedAsset {
                name: "GlowingCatInstaller-9.9.9-arm64.dmg".into(),
                url: "https://example/x.dmg".into(),
            }],
            false,
        );
        // force=false so the fresh cache is used (no network).
        let got = check(&mut state, PackageType::Macos, Arch::Arm64, false);
        assert!(got.is_none());
    }

    #[test]
    fn update_offered_when_cache_is_newer_with_asset() {
        let mut state = InstallState::default();
        // A clearly-newer tag than any realistic current version.
        state.put_cache(
            SELF_REPO,
            Some("v999.0.0".into()),
            vec![install::CachedAsset {
                name: "GlowingCatInstaller-999.0.0-arm64.dmg".into(),
                url: "https://example/new.dmg".into(),
            }],
            false,
        );
        let got = check(&mut state, PackageType::Macos, Arch::Arm64, false)
            .expect("a newer release with a matching asset should offer an update");
        assert_eq!(got.version, "v999.0.0");
        assert_eq!(got.asset_name, "GlowingCatInstaller-999.0.0-arm64.dmg");
    }

    #[test]
    fn no_update_when_newer_but_no_matching_asset() {
        let mut state = InstallState::default();
        // Newer, but only a Windows asset — no macOS/arm64 match.
        state.put_cache(
            SELF_REPO,
            Some("v999.0.0".into()),
            vec![install::CachedAsset {
                name: "GlowingCatInstaller-999.0.0-x64-Setup.exe".into(),
                url: "https://example/new.exe".into(),
            }],
            false,
        );
        let got = check(&mut state, PackageType::Macos, Arch::Arm64, false);
        assert!(got.is_none());
    }
}
