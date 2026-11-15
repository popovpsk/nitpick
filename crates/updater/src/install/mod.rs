use anyhow::{Context as _, Result};
use reqwest::StatusCode;
use reqwest::header::RANGE;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub struct Staged {
    #[cfg(target_os = "windows")]
    installer: PathBuf,
}

pub fn can_install(app_bundle: Option<&Path>) -> bool {
    if cfg!(target_os = "macos") {
        app_bundle.is_some_and(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
    } else {
        true
    }
}

pub async fn download_and_stage(
    url: String,
    digest: Option<String>,
    app_bundle: Option<PathBuf>,
) -> Result<Staged> {
    let digest = digest.as_deref();
    #[cfg(target_os = "macos")]
    {
        let bundle =
            app_bundle.ok_or_else(|| anyhow::anyhow!("running app path is unavailable"))?;
        macos::install(&url, digest, bundle).await?;
        Ok(Staged {})
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app_bundle;
        let installer = windows::download(&url, digest).await?;
        Ok(Staged { installer })
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app_bundle;
        linux::install(&url, digest).await?;
        Ok(Staged {})
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (url, digest, app_bundle);
        anyhow::bail!("auto-update is not supported on this platform")
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn appimage_path() -> Option<PathBuf> {
    linux::appimage_path()
}

#[cfg(target_os = "macos")]
pub(crate) fn relaunch(app_bundle: &Path) -> Result<()> {
    macos::relaunch(app_bundle)
}

impl Staged {
    pub fn finalize_on_quit(&self, relaunch: bool) {
        #[cfg(target_os = "windows")]
        windows::launch_installer(&self.installer, relaunch);
        #[cfg(not(target_os = "windows"))]
        let _ = relaunch;
    }
}

const DOWNLOAD_ATTEMPTS: u32 = 5;

const DOWNLOAD_RETRY_DELAY: Duration = Duration::from_secs(3);

const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(crate) async fn download_file(
    url: &str,
    dest: &Path,
    expected_digest: Option<&str>,
) -> Result<()> {
    download_with(&crate::github::client()?, url, dest, expected_digest).await
}

async fn download_with(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_digest: Option<&str>,
) -> Result<()> {
    let mut progress = Progress::default();
    let mut last_error = None;

    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        if attempt > 1 {
            tokio::time::sleep(DOWNLOAD_RETRY_DELAY).await;
        }

        if let Err(error) = fetch_into(client, url, dest, &mut progress).await {
            log::warn!(
                "updater: download attempt {attempt}/{DOWNLOAD_ATTEMPTS} failed at {} bytes: {error:#}",
                progress.received
            );
            last_error = Some(error);
            continue;
        }

        match verify_digest(progress.hasher.clone(), expected_digest) {
            Ok(()) => return Ok(()),
            Err(error) => {
                log::warn!("updater: {error:#}; re-downloading from scratch");
                progress.reset();
                last_error = Some(error);
            }
        }
    }

    let _ = tokio::fs::remove_file(dest).await;
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

#[derive(Default)]
struct Progress {
    hasher: sha2::Sha256,
    received: u64,
}

impl Progress {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn record(&mut self, bytes: &[u8]) {
        use sha2::Digest as _;
        self.hasher.update(bytes);
        self.received += bytes.len() as u64;
    }
}

async fn fetch_into(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: &mut Progress,
) -> Result<()> {
    let mut request = client.get(url).timeout(DOWNLOAD_TOTAL_TIMEOUT);
    if progress.received > 0 {
        request = request.header(RANGE, format!("bytes={}-", progress.received));
    }
    let mut response = request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .context("download request failed")?;

    if progress.received > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
        progress.reset();
    }

    let mut file = if progress.received > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .await
            .with_context(|| format!("opening {}", dest.display()))?
    } else {
        tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("creating {}", dest.display()))?
    };

    let mut transfer_error = None;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(error) = file.write_all(&chunk).await {
                    progress.reset();
                    return Err(error).context("writing downloaded file");
                }
                progress.record(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                transfer_error = Some(error);
                break;
            }
        }
    }
    if let Err(error) = file.flush().await {
        progress.reset();
        return Err(error).context("writing downloaded file");
    }
    match transfer_error {
        Some(error) => Err(error).context("downloading the update"),
        None => Ok(()),
    }
}

fn verify_digest(hasher: sha2::Sha256, expected_digest: Option<&str>) -> Result<()> {
    use sha2::Digest as _;

    let Some(digest) = expected_digest else {
        log::warn!("updater: release asset has no digest; skipping SHA-256 verification");
        return Ok(());
    };
    let Some(expected) = digest.strip_prefix("sha256:") else {
        log::warn!(
            "updater: release asset digest {digest:?} is not SHA-256; skipping verification"
        );
        return Ok(());
    };
    let actual = format!("{:x}", hasher.finalize());
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(expected),
        "downloaded file failed SHA-256 verification (expected {expected}, got {actual})"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
