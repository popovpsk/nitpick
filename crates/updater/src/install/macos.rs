use anyhow::{Context as _, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct Mounted {
    mount_point: PathBuf,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(&self.mount_point)
            .output();
    }
}

pub async fn install(url: &str, digest: Option<&str>, app_bundle: PathBuf) -> Result<()> {
    let dir = net_runtime::unblock(|| tempfile::Builder::new().prefix("slopcop-update").tempdir())
        .await
        .context("creating temp dir")?;
    let dmg = dir.path().join("slopcop.dmg");
    super::download_file(url, &dmg, digest).await?;
    net_runtime::unblock(move || replace_bundle(&dir, &dmg, &app_bundle)).await
}

fn replace_bundle(dir: &TempDir, dmg: &Path, app_bundle: &Path) -> Result<()> {
    let output = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-mountrandom"])
        .arg(dir.path())
        .arg(dmg)
        .output()
        .context("running hdiutil attach")?;
    anyhow::ensure!(
        output.status.success(),
        "hdiutil attach failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let canonical = std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
    let prefix = canonical.to_string_lossy();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mount_point = stdout
        .split_whitespace()
        .find(|token| token.starts_with(prefix.as_ref()))
        .map(PathBuf::from)
        .context("could not determine dmg mount point")?;
    let _mounted = Mounted {
        mount_point: mount_point.clone(),
    };

    let app_name = app_bundle
        .file_name()
        .context("running app has no file name")?;
    let mut source: OsString = mount_point.join(app_name).into();
    source.push("/");

    let output = Command::new("rsync")
        .args(["-a", "--delete", "--exclude", "Icon?"])
        .arg(&source)
        .arg(app_bundle)
        .output()
        .context("running rsync")?;
    anyhow::ensure!(
        output.status.success(),
        "rsync failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

pub fn relaunch(app_bundle: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt as _;

    let script = r#"
        while kill -0 "$0" 2>/dev/null; do
            sleep 0.1
        done
        for _ in {1..50}; do
            [ -z "$(lsappinfo find pid="$0" 2>/dev/null)" ] && break
            sleep 0.1
        done
        open "$1"
    "#;
    Command::new("/bin/bash")
        .arg("-c")
        .arg(script)
        .arg(std::process::id().to_string())
        .arg(app_bundle)
        .process_group(0)
        .spawn()
        .context("spawning relaunch script")?;
    Ok(())
}
