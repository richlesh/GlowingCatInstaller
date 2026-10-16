//! GitHub release API client, version parsing, and the flexible asset matcher.
//!
//! Asset filenames across the Glowing Cat projects are *not* consistent. Real
//! examples we must cope with:
//!   tagtiger-gui_1.2.0-1_amd64.deb         (debian, x64)
//!   tagtiger-x86_64-unknown-linux-gnu.rpm  (rpm, x64, target triple)
//!   TagTiger-aarch64-apple-darwin.dmg      (macos, arm64, target triple)
//!   NeuroPanther-Chat-1.11.2-arm64.exe     (windows, arm64)
//!   CPUSim64-2.8.0-x64.msi                 (windows, x64, msi not exe)
//!   ArchiveSphynx-2.2.0-arm64.dmg          (macos, arm64)
//!   Joshua-1.8.0-x64-setup.exe             (windows, x64)
//!
//! Rather than hardcode one scheme we tokenize the filename and score each
//! candidate against the requested (PackageType, Arch). The highest scoring
//! asset whose extension matches and whose architecture is not contradicted
//! wins.

use crate::platform::{Arch, PackageType};
use serde::Deserialize;
use std::time::Duration;

const USER_AGENT: &str = "GlowingCatInstaller/0.1 (+https://github.com/richlesh)";

/// A release asset as returned by the GitHub API.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // `size` is parsed for completeness / future UI use.
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// A GitHub release.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // name/prerelease/html_url parsed for completeness.
pub struct Release {
    #[serde(rename = "tag_name")]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(rename = "html_url", default)]
    pub html_url: String,
}

impl Release {
    /// Parse the semver-ish version out of the tag (strips a leading `v`).
    pub fn version(&self) -> Option<semver::Version> {
        parse_version(&self.tag_name)
    }

    /// The best asset for the given package type and architecture, if any.
    #[allow(dead_code)] // used by tests; the app uses best_named_asset on cached data.
    pub fn best_asset(&self, pkg: PackageType, arch: Arch) -> Option<&Asset> {
        best_asset(&self.assets, pkg, arch)
    }
}

/// Build a blocking HTTP client with sensible timeouts.
fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

/// Error type for release fetching.
#[derive(Debug)]
pub enum FetchError {
    /// The repo has no published (non-draft) release.
    NoRelease,
    /// Network / HTTP / decode failure.
    Http(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NoRelease => write!(f, "no published release"),
            FetchError::Http(e) => write!(f, "network error: {e}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetch the latest non-draft, non-prerelease release for `owner/repo`.
///
/// Falls back to the newest release of any kind if `/releases/latest` 404s but
/// the repo does have releases (some repos only publish prereleases).
pub fn latest_release(slug: &str) -> Result<Release, FetchError> {
    let c = client();

    // Primary: the "latest" endpoint (stable, non-draft, non-prerelease).
    let url = format!("https://api.github.com/repos/{slug}/releases/latest");
    let resp = c
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| FetchError::Http(e.to_string()))?;

    if resp.status().is_success() {
        return resp
            .json::<Release>()
            .map_err(|e| FetchError::Http(e.to_string()));
    }

    // Fallback: list all releases and take the newest usable one.
    let list_url = format!("https://api.github.com/repos/{slug}/releases?per_page=20");
    let list_resp = c
        .get(&list_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| FetchError::Http(e.to_string()))?;

    if !list_resp.status().is_success() {
        // 404 here means no releases at all.
        return Err(FetchError::NoRelease);
    }

    let releases: Vec<Release> = list_resp
        .json()
        .map_err(|e| FetchError::Http(e.to_string()))?;

    releases
        .into_iter()
        .filter(|r| !r.draft)
        .max_by(cmp_releases)
        .ok_or(FetchError::NoRelease)
}

/// Order two releases by parsed version, falling back to tag string order.
fn cmp_releases(a: &Release, b: &Release) -> std::cmp::Ordering {
    match (a.version(), b.version()) {
        (Some(va), Some(vb)) => va.cmp(&vb),
        _ => a.tag_name.cmp(&b.tag_name),
    }
}

/// Parse a version string that may have a leading `v` and may be missing a
/// patch component (e.g. "1.2" -> "1.2.0").
pub fn parse_version(raw: &str) -> Option<semver::Version> {
    let s = raw.trim();
    let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);

    // Keep only the leading numeric-dotted portion (drop any build suffix like
    // "-1" or "+meta" that isn't valid semver on its own).
    if let Ok(v) = semver::Version::parse(s) {
        return Some(v);
    }

    // Normalise "1" or "1.2" into full semver.
    let core: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Vec<&str> = core.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(semver::Version::new(major, minor, patch))
}

/// Returns true if `latest` is strictly newer than `installed`.
pub fn is_newer(installed: &str, latest: &str) -> bool {
    match (parse_version(installed), parse_version(latest)) {
        (Some(i), Some(l)) => l > i,
        // If we can't parse, be conservative: only report an upgrade when the
        // strings differ.
        _ => installed.trim() != latest.trim(),
    }
}

/// Score a candidate asset filename for the requested package type + arch.
/// Returns `None` if the asset fails a hard gate (wrong extension, or targets
/// the opposing architecture), otherwise `Some(score)` where higher is better.
pub fn score_asset_name(name: &str, pkg: PackageType, arch: Arch) -> Option<i32> {
    let lower = name.to_ascii_lowercase();

    // 1. Extension must match the requested package type. Hard gate.
    let ext_pos = pkg
        .extensions()
        .iter()
        .position(|ext| lower.ends_with(&format!(".{ext}")))?;

    // 2. Reject assets that explicitly target the *other* architecture.
    let has_opposing = arch
        .opposing_tokens()
        .iter()
        .any(|t| contains_token(&lower, t));
    if has_opposing {
        return None;
    }

    // 3. Score.
    let mut score = 0i32;

    // Preferred extension ordering (earlier = better).
    score += (pkg.extensions().len() as i32 - ext_pos as i32) * 2;

    // Matching architecture token present: strong signal.
    let has_arch = arch.tokens().iter().any(|t| contains_token(&lower, t));
    if has_arch {
        score += 20;
    } else {
        score += match arch {
            Arch::X64 => 6,
            Arch::Arm64 => 3,
        };
    }

    // "setup"/"installer" hints are nice for Windows.
    if matches!(pkg, PackageType::Windows)
        && (lower.contains("setup") || lower.contains("install"))
    {
        score += 2;
    }

    Some(score)
}

/// Score and select the best asset for the requested package type + arch.
#[allow(dead_code)] // used by tests; the app uses best_named_asset on cached data.
pub fn best_asset(assets: &[Asset], pkg: PackageType, arch: Arch) -> Option<&Asset> {
    let mut best: Option<(&Asset, i32)> = None;
    for asset in assets {
        if let Some(score) = score_asset_name(&asset.name, pkg, arch) {
            match best {
                Some((_, bs)) if bs >= score => {}
                _ => best = Some((asset, score)),
            }
        }
    }
    best.map(|(a, _)| a)
}

/// Select the best `(name, url)` pair from cached assets, using the same
/// scoring as [`best_asset`]. Returns borrowed references into `assets`.
pub fn best_named_asset(
    assets: &[(String, String)],
    pkg: PackageType,
    arch: Arch,
) -> Option<&(String, String)> {
    let mut best: Option<(&(String, String), i32)> = None;
    for pair in assets {
        if let Some(score) = score_asset_name(&pair.0, pkg, arch) {
            match best {
                Some((_, bs)) if bs >= score => {}
                _ => best = Some((pair, score)),
            }
        }
    }
    best.map(|(a, _)| a)
}

/// Token-boundary-aware substring check: ensures e.g. "x64" doesn't match
/// inside "x6400" and "arm64" matches in "...-arm64.dmg". We treat any
/// non-alphanumeric character (and string ends) as a boundary.
fn contains_token(haystack: &str, token: &str) -> bool {
    let bytes = haystack.as_bytes();
    let tb = token.as_bytes();
    if tb.is_empty() {
        return false;
    }
    let mut i = 0;
    while let Some(pos) = haystack[i..].find(token) {
        let start = i + pos;
        let end = start + tb.len();
        let left_ok = start == 0 || !is_tokenish(bytes[start - 1]);
        let right_ok = end == bytes.len() || !is_tokenish(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        i = start + 1;
        if i >= haystack.len() {
            break;
        }
    }
    false
}

/// Characters that count as part of a token (so adjacency breaks the boundary).
/// We intentionally treat digits+letters as tokenish but NOT `-`, `_`, `.`.
fn is_tokenish(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            download_url: format!("https://example/{name}"),
            size: 0,
        }
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("v1.2.0").unwrap(), semver::Version::new(1, 2, 0));
        assert_eq!(parse_version("0.4.0").unwrap(), semver::Version::new(0, 4, 0));
        assert_eq!(parse_version("1.11.2").unwrap(), semver::Version::new(1, 11, 2));
        assert_eq!(parse_version("1.2").unwrap(), semver::Version::new(1, 2, 0));
        assert!(parse_version("not-a-version").is_none());
    }

    #[test]
    fn newer_detection() {
        assert!(is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("v1.0.0", "1.0.1"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn token_boundaries() {
        assert!(contains_token("tagtiger-x64-setup.exe", "x64"));
        assert!(contains_token("spaceserval-x86_64-unknown-linux-gnu.rpm", "x86_64"));
        assert!(!contains_token("something-x6400.exe", "x64"));
        assert!(contains_token("foo-arm64.dmg", "arm64"));
    }

    #[test]
    fn match_debian_x64_underscore_scheme() {
        let assets = vec![
            a("tagtiger-gui_1.2.0-1_amd64.deb"),
            a("tagtiger-gui_1.2.0-1_arm64.deb"),
            a("tagtiger-x86_64-unknown-linux-gnu.rpm"),
        ];
        let got = best_asset(&assets, PackageType::Deb, Arch::X64).unwrap();
        assert_eq!(got.name, "tagtiger-gui_1.2.0-1_amd64.deb");
    }

    #[test]
    fn match_rpm_target_triple() {
        let assets = vec![
            a("tagtiger-aarch64-unknown-linux-gnu.rpm"),
            a("tagtiger-x86_64-unknown-linux-gnu.rpm"),
        ];
        let got = best_asset(&assets, PackageType::Rpm, Arch::Arm64).unwrap();
        assert_eq!(got.name, "tagtiger-aarch64-unknown-linux-gnu.rpm");
    }

    #[test]
    fn match_macos_arm64_dmg() {
        let assets = vec![
            a("ArchiveSphynx-2.2.0-arm64.dmg"),
            a("ArchiveSphynx-2.2.0-x64.dmg"),
            a("archivesphynx_2.2.0_amd64.deb"),
        ];
        let got = best_asset(&assets, PackageType::Macos, Arch::Arm64).unwrap();
        assert_eq!(got.name, "ArchiveSphynx-2.2.0-arm64.dmg");
    }

    #[test]
    fn match_windows_prefers_setup_exe_over_msi() {
        let assets = vec![
            a("Joshua-1.8.0-x64-setup.exe"),
            a("CPUSim64-2.8.0-x64.msi"),
        ];
        let got = best_asset(&assets, PackageType::Windows, Arch::X64).unwrap();
        assert_eq!(got.name, "Joshua-1.8.0-x64-setup.exe");
    }

    #[test]
    fn windows_msi_when_only_option() {
        let assets = vec![a("LayoutLynx-1.3.0-x64.msi"), a("LayoutLynx-1.3.0-arm64.msi")];
        let got = best_asset(&assets, PackageType::Windows, Arch::X64).unwrap();
        assert_eq!(got.name, "LayoutLynx-1.3.0-x64.msi");
    }

    #[test]
    fn no_match_for_absent_package_type() {
        let assets = vec![a("foo-x64.dmg")];
        assert!(best_asset(&assets, PackageType::Deb, Arch::X64).is_none());
    }

    #[test]
    fn macos_target_triple_scheme() {
        let assets = vec![
            a("TagTiger-aarch64-apple-darwin.dmg"),
        ];
        let got = best_asset(&assets, PackageType::Macos, Arch::Arm64).unwrap();
        assert_eq!(got.name, "TagTiger-aarch64-apple-darwin.dmg");
        // And an x64 request should NOT pick the arm64 asset.
        assert!(best_asset(&assets, PackageType::Macos, Arch::X64).is_none());
    }
}
