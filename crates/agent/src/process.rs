use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};
use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;
use tokio::time::error::Elapsed;

use crate::lines::Lines;
use crate::{Error, HarnessKind};

pub const SCRUBBED_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_AGENT_SDK_VERSION",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

pub const SCRUBBED_PREFIXES: &[&str] = &["ORCA_"];

const SKIPPED_SHELL_VARS: &[&str] = &["_", "PWD", "OLDPWD", "SHLVL"];

const HOME_DIRS: &[&str] = &[
    ".local/bin",
    ".claude/local",
    ".opencode/bin",
    ".codex/bin",
    ".npm-global/bin",
];

const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

const LOGIN_SHELLS: &[&str] = &["sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "yash"];

pub const BEGIN_MARKER: &str = "__NITPICK_ENV_BEGIN__";
pub const END_MARKER: &str = "__NITPICK_ENV_END__";

#[cfg(unix)]
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const SNAPSHOT_WAIT: Duration = Duration::from_secs(7);
const STDERR_TAIL_BYTES: usize = 4096;
const STDERR_DRAIN: Duration = Duration::from_millis(500);
const KILL_WAIT: Duration = Duration::from_secs(2);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_OUTPUT_BYTES: u64 = 64 * 1024;

pub fn split_words(template: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in template.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

pub fn login_shell(shell: Option<&str>) -> String {
    shell
        .map(str::trim)
        .filter(|shell| shell.starts_with('/'))
        .filter(|shell| {
            Path::new(shell)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| LOGIN_SHELLS.contains(&name))
        })
        .unwrap_or("/bin/sh")
        .to_string()
}

pub fn parse_snapshot(text: &str) -> Option<Vec<(String, String)>> {
    let mut lines = text.lines();
    lines.find(|line| line.trim_end() == BEGIN_MARKER)?;
    let mut vars: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.trim_end() == END_MARKER {
            return Some(vars);
        }
        match split_assignment(line) {
            Some((name, value)) => vars.push((name.to_owned(), value.to_owned())),
            None => {
                if let Some((_, value)) = vars.last_mut() {
                    value.push('\n');
                    value.push_str(line);
                }
            }
        }
    }
    None
}

fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some((name, value))
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub vars: Vec<(String, String)>,
}

static SNAPSHOT: OnceLock<Snapshot> = OnceLock::new();

pub fn snapshot() -> &'static Snapshot {
    SNAPSHOT.get_or_init(capture)
}

pub fn prewarm() {
    drop(net_runtime::handle().spawn_blocking(|| {
        snapshot();
    }));
}

#[cfg(unix)]
fn capture() -> Snapshot {
    let shell = login_shell(std::env::var("SHELL").ok().as_deref());
    let (sender, receiver) = std::sync::mpsc::channel();
    let task = net_runtime::handle().spawn(async move {
        let _ = sender.send(capture_from(&shell).await);
    });
    let vars = receiver.recv_timeout(SNAPSHOT_WAIT).ok().flatten();
    task.abort();
    match vars {
        Some(vars) => Snapshot { vars },
        None => {
            log::warn!("the login shell did not report its environment; using the app's own");
            Snapshot::default()
        }
    }
}

#[cfg(not(unix))]
fn capture() -> Snapshot {
    Snapshot::default()
}

#[cfg(unix)]
pub async fn capture_from(shell: &str) -> Option<Vec<(String, String)>> {
    let script = format!("printf '%s\\n' {BEGIN_MARKER}; env; printf '%s\\n' {END_MARKER}");
    let invocation = Invocation::bare(
        PathBuf::from(shell),
        vec![OsString::from("-lic"), OsString::from(script)],
        std::env::temp_dir(),
        Environment::from_process(),
    );
    let (mut process, pipes) = match Process::spawn(&invocation) {
        Ok(spawned) => spawned,
        Err(error) => {
            log::warn!("starting the login shell: {error}");
            return None;
        }
    };
    drop(pipes.stdin);
    let mut lines = Lines::new(pipes.stdout);
    let mut text = String::new();
    let read = tokio::time::timeout(SNAPSHOT_TIMEOUT, async {
        while let Ok(Some(line)) = lines.next_line().await {
            let done = line == END_MARKER;
            text.push_str(&line);
            text.push('\n');
            if done {
                break;
            }
        }
    })
    .await;
    process.shutdown(Grace::IMMEDIATE).await;
    if read.is_err() {
        log::warn!(
            "the login shell took longer than {SNAPSHOT_TIMEOUT:?} to report its environment"
        );
        return None;
    }
    parse_snapshot(&text)
}

fn same_key(a: &OsStr, b: &OsStr) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn is_path_key(key: &OsStr) -> bool {
    same_key(key, OsStr::new("PATH"))
}

pub fn is_scrubbed(key: &OsStr) -> bool {
    let Some(key) = key.to_str() else {
        return false;
    };
    SCRUBBED_ENV.contains(&key)
        || SCRUBBED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

pub fn known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = home
        .map(|home| HOME_DIRS.iter().map(|dir| home.join(dir)).collect())
        .unwrap_or_default();
    dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
    dirs
}

#[derive(Debug, Clone)]
pub struct Environment {
    vars: Vec<(OsString, OsString)>,
    path: OsString,
}

impl Environment {
    pub fn from_process() -> Self {
        Self::merge(
            std::env::vars_os().collect(),
            &[],
            dirs::home_dir().as_deref(),
        )
    }

    pub fn current() -> Self {
        Self::merge(
            std::env::vars_os().collect(),
            &snapshot().vars,
            dirs::home_dir().as_deref(),
        )
    }

    pub fn merge(
        process: Vec<(OsString, OsString)>,
        shell: &[(String, String)],
        home: Option<&Path>,
    ) -> Self {
        let mut vars: Vec<(OsString, OsString)> = process
            .into_iter()
            .filter(|(key, _)| !is_scrubbed(key))
            .collect();
        let own_path = vars
            .iter()
            .find(|(key, _)| is_path_key(key))
            .map(|(_, value)| value.clone());
        let mut entries: Vec<PathBuf> = Vec::new();
        if let Some((_, value)) = shell.iter().find(|(key, _)| is_path_key(OsStr::new(key))) {
            entries.extend(std::env::split_paths(value));
        }
        if let Some(value) = &own_path {
            entries.extend(std::env::split_paths(value));
        }
        entries.extend(known_dirs(home));
        let mut seen = HashSet::new();
        entries.retain(|entry| !entry.as_os_str().is_empty() && seen.insert(entry.clone()));

        for (key, value) in shell {
            let name = OsStr::new(key);
            let skipped = is_path_key(name)
                || is_scrubbed(name)
                || SKIPPED_SHELL_VARS.contains(&key.as_str())
                || vars.iter().any(|(existing, _)| same_key(existing, name));
            if !skipped {
                vars.push((OsString::from(key), OsString::from(value)));
            }
        }

        let path = std::env::join_paths(&entries).unwrap_or_else(|_| own_path.unwrap_or_default());
        vars.retain(|(key, _)| !is_path_key(key));
        vars.push((OsString::from("PATH"), path.clone()));
        Self { vars, path }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let name = OsStr::new(key);
        self.vars.retain(|(existing, _)| !same_key(existing, name));
        if is_path_key(name) {
            self.path = OsString::from(value);
        }
        self.vars.push((OsString::from(key), OsString::from(value)));
    }

    pub fn get(&self, key: &str) -> Option<&OsStr> {
        let name = OsStr::new(key);
        self.vars
            .iter()
            .find(|(existing, _)| same_key(existing, name))
            .map(|(_, value)| value.as_os_str())
    }

    pub fn path(&self) -> &OsStr {
        &self.path
    }

    pub fn vars(&self) -> &[(OsString, OsString)] {
        &self.vars
    }
}

pub fn extensions_for(program: &str, pathext: &str) -> Vec<String> {
    let listed: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(str::to_owned)
        .collect();
    if listed.is_empty() || Path::new(program).extension().is_some() {
        vec![String::new()]
    } else {
        listed
    }
}

pub fn candidates(dir: &Path, program: &str, extensions: &[String]) -> Vec<PathBuf> {
    extensions
        .iter()
        .map(|extension| dir.join(format!("{program}{extension}")))
        .collect()
}

fn is_runnable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn resolve_program(program: &str, path: &OsStr, pathext: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.chars().any(std::path::is_separator) {
        return Some(PathBuf::from(program));
    }
    let extensions = extensions_for(program, pathext);
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| {
            candidates(&dir, program, &extensions)
                .into_iter()
                .find(|candidate| is_runnable(candidate))
        })
}

fn pathext() -> String {
    if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
    } else {
        String::new()
    }
}

#[derive(Debug, Clone)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Environment,
}

impl Invocation {
    pub fn new(
        command: &str,
        extra_args: Vec<String>,
        extra_env: &[(String, String)],
        cwd: &Path,
    ) -> Result<Self, Error> {
        Self::resolve_in(command, extra_args, extra_env, Environment::current(), cwd)
    }

    pub fn resolve_in(
        command: &str,
        extra_args: Vec<String>,
        extra_env: &[(String, String)],
        mut env: Environment,
        cwd: &Path,
    ) -> Result<Self, Error> {
        let mut words = split_words(command);
        if words.is_empty() {
            return Err(Error::EmptyCommand);
        }
        let name = words.remove(0);
        let program =
            resolve_program(&name, env.path(), &pathext()).ok_or(Error::NotFound(name))?;
        for (key, value) in extra_env {
            env.set(key, value);
        }
        let args = words
            .into_iter()
            .chain(extra_args)
            .map(OsString::from)
            .collect();
        Ok(Self {
            program,
            args,
            cwd: cwd.to_path_buf(),
            env,
        })
    }

    pub fn bare(program: PathBuf, args: Vec<OsString>, cwd: PathBuf, env: Environment) -> Self {
        Self {
            program,
            args,
            cwd,
            env,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StderrTail(Arc<Mutex<String>>);

impl StderrTail {
    pub fn push_line(&self, line: &str) {
        let mut text = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        text.push_str(line);
        text.push('\n');
        if text.len() > STDERR_TAIL_BYTES {
            let mut cut = text.len() - STDERR_TAIL_BYTES;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text.drain(..cut);
        }
    }

    pub fn text(&self) -> String {
        let text = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        text.trim().to_owned()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Grace {
    pub stdin: Duration,
    pub term: Duration,
}

impl Grace {
    pub const IMMEDIATE: Grace = Grace {
        stdin: Duration::ZERO,
        term: Duration::ZERO,
    };
}

impl Default for Grace {
    fn default() -> Self {
        Self {
            stdin: Duration::from_secs(3),
            term: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Exit {
    pub status: Option<ExitStatus>,
    pub stderr: String,
}

#[derive(Debug)]
pub struct Pipes {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

pub struct Process {
    child: Box<dyn ChildWrapper>,
    stderr: StderrTail,
    stderr_task: Option<JoinHandle<()>>,
}

impl Process {
    pub fn spawn(invocation: &Invocation) -> Result<(Process, Pipes), Error> {
        let _entered = net_runtime::handle().enter();
        let mut wrap = CommandWrap::with_new(&invocation.program, |command| {
            command
                .args(&invocation.args)
                .current_dir(&invocation.cwd)
                .env_clear()
                .envs(
                    invocation
                        .env
                        .vars()
                        .iter()
                        .map(|(key, value)| (key, value)),
                )
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
        });
        wrap.wrap(KillOnDrop);
        #[cfg(unix)]
        {
            wrap.wrap(ProcessGroup::leader());
        }
        #[cfg(windows)]
        {
            wrap.wrap(CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ));
            wrap.wrap(JobObject);
        }
        let mut child = wrap
            .spawn()
            .map_err(|source| spawn_error(invocation, source))?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (
            child.stdin().take(),
            child.stdout().take(),
            child.stderr().take(),
        ) else {
            let _ = child.start_kill();
            return Err(Error::Io(std::io::Error::other(
                "the child process has no standard streams",
            )));
        };
        let tail = StderrTail::default();
        let stderr_task = net_runtime::handle().spawn(read_stderr(stderr, tail.clone()));
        let process = Process {
            child,
            stderr: tail,
            stderr_task: Some(stderr_task),
        };
        Ok((process, Pipes { stdin, stdout }))
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr.text()
    }

    pub async fn wait(&mut self) -> Exit {
        let status = match self.child.wait().await {
            Ok(status) => Some(status),
            Err(error) => {
                log::warn!("waiting for the child process: {error}");
                None
            }
        };
        self.finish(status).await
    }

    pub async fn shutdown(&mut self, grace: Grace) -> Exit {
        let mut waited = self.wait_within(grace.stdin).await;
        #[cfg(unix)]
        {
            const SIGTERM: i32 = 15;
            if waited.is_err() {
                if let Err(error) = self.child.signal(SIGTERM) {
                    log::debug!("sending SIGTERM to the process group: {error}");
                }
                waited = self.wait_within(grace.term).await;
            }
        }
        if waited.is_err() {
            if let Err(error) = self.child.start_kill() {
                log::warn!("killing the process group: {error}");
            }
            waited = self.wait_within(KILL_WAIT).await;
        }
        let status = waited.ok().flatten();
        self.finish(status).await
    }

    async fn wait_within(&mut self, limit: Duration) -> Result<Option<ExitStatus>, Elapsed> {
        let waited = tokio::time::timeout(limit, self.child.wait()).await?;
        Ok(match waited {
            Ok(status) => Some(status),
            Err(error) => {
                log::warn!("waiting for the child process: {error}");
                None
            }
        })
    }

    async fn finish(&mut self, status: Option<ExitStatus>) -> Exit {
        let _ = self.child.start_kill();
        drain(&mut self.stderr_task, STDERR_DRAIN).await;
        Exit {
            status,
            stderr: self.stderr.text(),
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }
}

pub(crate) async fn drain(task: &mut Option<JoinHandle<()>>, limit: Duration) {
    let Some(handle) = task.as_mut() else {
        return;
    };
    let drained = tokio::time::timeout(limit, &mut *handle).await.is_ok();
    if let Some(handle) = task.take()
        && !drained
    {
        handle.abort();
    }
}

fn spawn_error(invocation: &Invocation, source: std::io::Error) -> Error {
    let program = invocation.program.display().to_string();
    if source.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound(program)
    } else {
        Error::Spawn { program, source }
    }
}

async fn read_stderr(stderr: ChildStderr, tail: StderrTail) {
    let mut lines = Lines::new(stderr);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                log::trace!(target: "agent::stderr", "{line}");
                tail.push_line(&line);
            }
            Ok(None) => break,
            Err(error) => {
                log::debug!("reading the child's stderr: {error}");
                break;
            }
        }
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn on_runtime<T, F>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut task = AbortOnDrop(net_runtime::handle().spawn(future));
    match (&mut task.0).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

pub async fn probe_version(command: &str, kind: HarnessKind) -> Result<String, Error> {
    log::debug!("probing {kind:?} with `{command} --version`");
    let env = net_runtime::unblock(Environment::current).await;
    let invocation = Invocation::resolve_in(
        command,
        vec!["--version".to_owned()],
        &[],
        env,
        &std::env::temp_dir(),
    )?;
    let (mut process, pipes) = Process::spawn(&invocation)?;
    drop(pipes.stdin);
    let mut stdout = pipes.stdout.take(PROBE_OUTPUT_BYTES);
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(PROBE_TIMEOUT, stdout.read_to_end(&mut bytes)).await;
    let grace = if read.is_ok() {
        Grace {
            stdin: Duration::from_secs(1),
            term: Duration::from_millis(500),
        }
    } else {
        Grace::IMMEDIATE
    };
    let exit = process.shutdown(grace).await;
    let fail = |detail: String| Error::Probe {
        command: command.to_owned(),
        detail,
    };
    if read.is_err() {
        return Err(fail(format!("no answer within {PROBE_TIMEOUT:?}")));
    }
    let succeeded = exit.status.is_some_and(|status| status.success());
    let output = String::from_utf8_lossy(&bytes);
    let first_line = |text: &str| {
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    };
    let version = first_line(&output).or_else(|| first_line(&exit.stderr));
    match (succeeded, version) {
        (true, Some(version)) => Ok(version),
        (true, None) => Err(fail("it printed nothing".to_owned())),
        (false, _) => Err(fail(match first_line(&exit.stderr) {
            Some(line) => line,
            None => match exit.status {
                Some(status) => status.to_string(),
                None => "it did not exit".to_owned(),
            },
        })),
    }
}
