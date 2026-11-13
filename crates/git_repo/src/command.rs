use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use crate::GitError;

pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

const BASE_CONFIG: [&str; 4] = ["-c", "core.quotepath=false", "-c", "color.ui=false"];

const SCRUBBED_ENV: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command<S: AsRef<OsStr>>(root: &Path, args: &[S]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(BASE_CONFIG)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in SCRUBBED_ENV {
        command.env_remove(name);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

pub(crate) fn display_args<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|arg| arg.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn output<S: AsRef<OsStr>>(root: &Path, args: &[S]) -> Result<Output, GitError> {
    log::debug!("git {}", display_args(args));
    Ok(command(root, args).output()?)
}

pub(crate) fn run<S: AsRef<OsStr>>(root: &Path, args: &[S]) -> Result<Vec<u8>, GitError> {
    let output = output(root, args)?;
    check(output, args)
}

pub(crate) async fn output_async<S: AsRef<OsStr>>(
    root: &Path,
    args: &[S],
    timeout: Duration,
) -> Result<Output, GitError> {
    log::debug!("git {} (async)", display_args(args));
    let mut command = tokio::process::Command::from(command(root, args));
    command.kill_on_drop(true);
    let child = command.spawn()?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => Ok(result?),
        Err(_) => {
            let args = display_args(args);
            log::warn!("git {args} timed out after {} s", timeout.as_secs());
            Err(GitError::TimedOut { args })
        }
    }
}

pub(crate) async fn run_async<S: AsRef<OsStr>>(
    root: &Path,
    args: &[S],
    timeout: Duration,
) -> Result<Vec<u8>, GitError> {
    let output = output_async(root, args, timeout).await?;
    check(output, args)
}

pub(crate) fn check<S: AsRef<OsStr>>(output: Output, args: &[S]) -> Result<Vec<u8>, GitError> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_error(&output, args))
    }
}

pub(crate) fn command_error<S: AsRef<OsStr>>(output: &Output, args: &[S]) -> GitError {
    GitError::Command {
        args: display_args(args),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches(['\n', '\r'])
        .to_string()
}

pub(crate) fn records(bytes: &[u8], fields: usize) -> Vec<Vec<String>> {
    let text = String::from_utf8_lossy(bytes);
    let mut parts: Vec<&str> = text.split('\0').collect();
    if parts.last().is_some_and(|last| last.trim().is_empty()) {
        parts.pop();
    }
    parts
        .chunks(fields)
        .filter(|chunk| chunk.len() == fields)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if index == 0 {
                        field.trim_start_matches(['\n', '\r']).to_string()
                    } else {
                        field.to_string()
                    }
                })
                .collect()
        })
        .collect()
}

pub(crate) fn checked_arg(value: &str) -> Result<&str, GitError> {
    if value.is_empty() || value.starts_with('-') {
        Err(GitError::Parse(format!("invalid git argument {value:?}")))
    } else {
        Ok(value)
    }
}
