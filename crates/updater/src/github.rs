use anyhow::{Context as _, Result};
use reqwest::StatusCode;
use reqwest::header::ACCEPT;
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

use crate::version;

const REPO: &str = "popovpsk/slopcop";
const USER_AGENT: &str = "slopcop-updater";
const API_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub struct Found {
    pub version: Version,
    pub url: String,
    pub digest: Option<String>,
}

pub(crate) fn client() -> Result<reqwest::Client> {
    http_client::builder(USER_AGENT)?
        .https_only(true)
        .no_gzip()
        .build()
        .context("building the HTTP client")
}

pub async fn fetch_latest() -> Result<Option<Found>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = client()?
        .get(&url)
        .timeout(API_TIMEOUT)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("GitHub releases request failed")?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .context("GitHub releases request failed")?;

    let body = response
        .bytes()
        .await
        .context("reading GitHub releases response")?;
    let release: Release =
        serde_json::from_slice(&body).context("parsing GitHub releases response")?;

    let version = version::parse(&release.tag_name)?;
    let asset = select_asset(&release.assets).context("no release asset matches this platform")?;
    Ok(Some(Found {
        version,
        url: asset.browser_download_url.clone(),
        digest: asset.digest.clone(),
    }))
}

fn select_asset(assets: &[Asset]) -> Option<&Asset> {
    assets.iter().find(|asset| asset_matches(&asset.name))
}

#[cfg(any(target_os = "macos", test))]
fn macos_asset(name: &str) -> bool {
    name.ends_with(".dmg")
}

#[cfg(any(target_os = "windows", test))]
fn windows_asset(name: &str, arch: &str) -> bool {
    match arch {
        "x86_64" => name.ends_with("_x64-setup.exe"),
        "aarch64" => name.ends_with("_arm64-installer.exe"),
        _ => false,
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_asset(name: &str, arch: &str) -> bool {
    name.ends_with(".AppImage") && name.contains(arch)
}

#[cfg(target_os = "macos")]
fn asset_matches(name: &str) -> bool {
    macos_asset(name)
}

#[cfg(target_os = "windows")]
fn asset_matches(name: &str) -> bool {
    windows_asset(name, std::env::consts::ARCH)
}

#[cfg(target_os = "linux")]
fn asset_matches(name: &str) -> bool {
    linux_asset(name, std::env::consts::ARCH)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn asset_matches(_name: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE_ASSETS: &[&str] = &[
        "slopcop_1.0.0_aarch64-pacman.tar.gz",
        "slopcop_1.0.0_aarch64.AppImage",
        "slopcop_1.0.0_aarch64.AppImage.zsync",
        "Slopcop_1.0.0_aarch64.dmg",
        "slopcop_1.0.0_amd64.deb",
        "slopcop_1.0.0_arm64-installer.exe",
        "slopcop_1.0.0_arm64-portable.zip",
        "slopcop_1.0.0_arm64.deb",
        "slopcop_1.0.0_x64-portable.zip",
        "slopcop_1.0.0_x64-setup.exe",
        "slopcop_1.0.0_x86_64-pacman.tar.gz",
        "slopcop_1.0.0_x86_64.AppImage",
        "slopcop_1.0.0_x86_64.AppImage.zsync",
    ];

    fn assets() -> Vec<Asset> {
        RELEASE_ASSETS
            .iter()
            .map(|name| Asset {
                name: (*name).to_string(),
                browser_download_url: format!("https://example.invalid/{name}"),
                digest: None,
            })
            .collect()
    }

    #[test]
    fn exactly_one_setup_exe_per_release() {
        let setups: Vec<&&str> = RELEASE_ASSETS
            .iter()
            .filter(|name| name.ends_with("-setup.exe"))
            .collect();
        assert_eq!(setups, vec![&"slopcop_1.0.0_x64-setup.exe"]);
    }

    fn only_match(rule: impl Fn(&str) -> bool) -> &'static str {
        let matched: Vec<&&str> = RELEASE_ASSETS.iter().filter(|name| rule(name)).collect();
        assert_eq!(matched.len(), 1, "expected one match, got {matched:?}");
        matched[0]
    }

    #[test]
    fn windows_x86_64_never_picks_the_arm64_installer() {
        assert_eq!(
            only_match(|name| windows_asset(name, "x86_64")),
            "slopcop_1.0.0_x64-setup.exe"
        );
    }

    #[test]
    fn windows_aarch64_picks_the_arm64_installer() {
        assert_eq!(
            only_match(|name| windows_asset(name, "aarch64")),
            "slopcop_1.0.0_arm64-installer.exe"
        );
    }

    #[test]
    fn macos_picks_the_only_dmg() {
        assert_eq!(only_match(macos_asset), "Slopcop_1.0.0_aarch64.dmg");
    }

    #[test]
    fn linux_picks_the_appimage_for_its_arch() {
        assert_eq!(
            only_match(|name| linux_asset(name, "x86_64")),
            "slopcop_1.0.0_x86_64.AppImage"
        );
        assert_eq!(
            only_match(|name| linux_asset(name, "aarch64")),
            "slopcop_1.0.0_aarch64.AppImage"
        );
    }

    #[test]
    fn select_asset_is_order_independent() {
        let mut assets = assets();
        assets.reverse();
        let reversed = select_asset(&assets).map(|asset| asset.name.clone());
        let forward = select_asset(&self::assets()).map(|asset| asset.name.clone());
        assert_eq!(reversed, forward);
        assert!(forward.is_some());
    }
}
