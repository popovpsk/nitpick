use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use git_repo::Remote;
use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, SharedString, Task, actions};
use gpui_component::{
    diff::DiffSide,
    menu::{PopupMenu, PopupMenuItem},
    notification::Notification,
};
use review_core::{Change, FileStatus, LineKind};

use super::{ReviewSource, ReviewView};
use crate::localization::tr;
use crate::settings_store::settings;

actions!(review, [CopyPath, OpenInEditor]);

const FAILURE_WINDOW: Duration = Duration::from_secs(5);
const POLL_FAST: Duration = Duration::from_millis(100);
const POLL_SLOW: Duration = Duration::from_secs(2);
const FILE_VAR: &str = "LGTM_ISH_FILE";
const LINE_VAR: &str = "LGTM_ISH_LINE";
const ROOT_VAR: &str = "LGTM_ISH_ROOT";
const JETBRAINS: [(&str, &str, &str); 5] = [
    ("GoLand", "GoLand.app", "goland"),
    ("IntelliJ IDEA", "IntelliJ IDEA.app", "idea"),
    ("RustRover", "RustRover.app", "rustrover"),
    ("PyCharm", "PyCharm.app", "pycharm"),
    ("WebStorm", "WebStorm.app", "webstorm"),
];

pub fn editor_presets(os: HostOs) -> Vec<(&'static str, String)> {
    let mut presets = vec![
        ("VS Code", "code {root} -g {file}:{line}".to_string()),
        ("Cursor", "cursor {root} -g {file}:{line}".to_string()),
        ("Zed", "zed {root} {file}:{line}".to_string()),
    ];
    presets.extend(JETBRAINS.iter().map(|(name, app, script)| {
        let template = match os {
            HostOs::Mac => format!("open -na \"{app}\" --args {{root}} --line {{line}} {{file}}"),
            HostOs::Linux | HostOs::Windows => {
                format!("{script} {{root}} --line {{line}} {{file}}")
            }
        };
        (*name, template)
    }));
    presets
}
const LOGIN_SHELLS: [&str; 8] = ["sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "yash"];

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitLab,
    GitHub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWeb {
    pub base: String,
    pub forge: Forge,
}

pub fn project_web_from_mr(web_url: &str) -> Option<ProjectWeb> {
    let url = web_url.trim().trim_end_matches('/');
    let (base, _) = url
        .rsplit_once("/-/merge_requests/")
        .or_else(|| url.rsplit_once("/merge_requests/"))?;
    let (_, rest) = base.split_once("://")?;
    rest.contains('/').then(|| ProjectWeb {
        base: base.to_string(),
        forge: Forge::GitLab,
    })
}

pub fn project_web_from_remotes(remotes: &[Remote]) -> Option<ProjectWeb> {
    let remote = remotes
        .iter()
        .find(|r| r.name == "origin")
        .or_else(|| remotes.first())?;
    let location = git_repo::parse_remote_url(&remote.url)?;
    let host = location.host.to_ascii_lowercase();
    let forge = if host == "github.com" || host == "www.github.com" {
        Forge::GitHub
    } else {
        Forge::GitLab
    };
    Some(ProjectWeb {
        base: format!("{}/{}", location.https_base, location.path),
        forge,
    })
}

pub fn blob_url(web: &ProjectWeb, sha: &str, path: &str, line: Option<u32>) -> String {
    let blob = match web.forge {
        Forge::GitLab => "-/blob",
        Forge::GitHub => "blob",
    };
    let mut url = format!("{}/{blob}/{sha}/", web.base);
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(byte as char)
            }
            _ => {
                let _ = write!(url, "%{byte:02X}");
            }
        }
    }
    if let Some(line) = line {
        let _ = write!(url, "#L{line}");
    }
    url
}

pub fn change_line(change: &Change) -> u32 {
    let line = match (change.first, change.anchor) {
        (LineKind::Added { new } | LineKind::Context { new, .. }, _) => new,
        (LineKind::Removed { old }, LineKind::Context { old: above, new }) => {
            new + old.saturating_sub(above)
        }
        (LineKind::Removed { .. }, _) => 1,
    };
    line.max(1)
}

pub fn pick_line(
    cursor: Option<&Change>,
    scrolled: bool,
    selection: Option<u32>,
    first: Option<&Change>,
) -> u32 {
    let at_cursor = cursor.map(change_line);
    let line = if scrolled {
        selection.or(at_cursor)
    } else {
        at_cursor.or(selection)
    };
    line.or_else(|| first.map(change_line)).unwrap_or(1).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Mac,
    Linux,
    Windows,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::Mac
        } else {
            HostOs::Linux
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, String)>,
    pub check_exit: bool,
}

impl Invocation {
    fn plain(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: Vec::new(),
            check_exit: true,
        }
    }

    pub fn display(&self) -> String {
        match self.args.as_slice() {
            [flag, script] if flag == "-lc" => {
                self.env.iter().fold(script.clone(), |text, (key, value)| {
                    text.replace(&format!("'\"${key}\"'"), value)
                        .replace(&format!("\"${key}\""), value)
                        .replace(&format!("${key}"), value)
                })
            }
            args => std::iter::once(self.program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

pub fn editor_invocation(
    template: &str,
    root: &str,
    file: &str,
    line: u32,
    os: HostOs,
    shell: Option<&str>,
) -> Invocation {
    let template = template.trim();
    let line_text = line.to_string();
    let wants_file = template.contains("{file}");
    if template.is_empty() {
        return match os {
            HostOs::Mac => Invocation::plain("open", &[file]),
            HostOs::Linux => Invocation::plain("xdg-open", &[file]),
            HostOs::Windows => Invocation {
                check_exit: false,
                ..Invocation::plain("explorer.exe", &[file])
            },
        };
    }
    match os {
        HostOs::Windows => {
            let mut words = split_words(template)
                .into_iter()
                .map(|word| substitute(&word, root, file, &line_text));
            let program = words.next().unwrap_or_default();
            let mut args: Vec<String> = words.collect();
            if !wants_file {
                args.push(file.to_string());
            }
            Invocation {
                program,
                args,
                env: Vec::new(),
                check_exit: true,
            }
        }
        HostOs::Mac | HostOs::Linux => {
            let mut script = substitute_shell(template);
            if !wants_file {
                let _ = write!(script, " \"${FILE_VAR}\"");
            }
            Invocation {
                program: login_shell(shell),
                args: vec!["-lc".to_string(), script],
                env: vec![
                    (FILE_VAR, file.to_string()),
                    (LINE_VAR, line_text),
                    (ROOT_VAR, root.to_string()),
                ],
                check_exit: true,
            }
        }
    }
}

fn placeholder(rest: &str) -> Option<(&'static str, &str)> {
    rest.strip_prefix("{file}")
        .map(|after| (FILE_VAR, after))
        .or_else(|| rest.strip_prefix("{line}").map(|after| (LINE_VAR, after)))
        .or_else(|| rest.strip_prefix("{root}").map(|after| (ROOT_VAR, after)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

fn substitute_shell(template: &str) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut quote = Quote::None;
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if let Some((var, after)) = placeholder(rest) {
            let _ = match quote {
                Quote::None => write!(out, "\"${var}\""),
                Quote::Single => write!(out, "'\"${var}\"'"),
                Quote::Double => write!(out, "${var}"),
            };
            rest = after;
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
        match (quote, c) {
            (Quote::None | Quote::Double, '\\') => {
                if let Some(next) = rest.chars().next() {
                    out.push(next);
                    rest = &rest[next.len_utf8()..];
                }
            }
            (Quote::None, '\'') => quote = Quote::Single,
            (Quote::None, '"') => quote = Quote::Double,
            (Quote::Single, '\'') | (Quote::Double, '"') => quote = Quote::None,
            _ => {}
        }
    }
    out
}

fn split_words(template: &str) -> Vec<String> {
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

fn substitute(template: &str, root: &str, file: &str, line: &str) -> String {
    let mut out = String::with_capacity(template.len() + file.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match placeholder(tail) {
            Some((var, after)) => {
                out.push_str(match var {
                    FILE_VAR => file,
                    ROOT_VAR => root,
                    _ => line,
                });
                rest = after;
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn login_shell(shell: Option<&str>) -> String {
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

pub fn find_program(program: &str, path: &OsStr, pathext: &str) -> Option<PathBuf> {
    if program.is_empty()
        || program.contains(['/', '\\', ':'])
        || Path::new(program).extension().is_some()
    {
        return None;
    }
    std::env::split_paths(path).find_map(|dir| {
        pathext
            .split(';')
            .map(str::trim)
            .filter(|ext| !ext.is_empty())
            .map(|ext| dir.join(format!("{program}{ext}")))
            .find(|candidate| candidate.is_file())
    })
}

fn spawn(invocation: &Invocation, dir: &Path) -> std::io::Result<Child> {
    let resolved = if cfg!(windows) {
        std::env::var_os("PATH").and_then(|path| {
            let pathext =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            find_program(&invocation.program, &path, &pathext)
        })
    } else {
        None
    };
    let mut command = match resolved {
        Some(program) => Command::new(program),
        None => Command::new(&invocation.program),
    };
    command
        .args(&invocation.args)
        .envs(invocation.env.iter().map(|(key, value)| (key, value)))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}

enum Launch {
    Failed(String),
    Exited(ExitStatus),
    Done,
    Running(Child),
}

pub struct WebLink {
    project: Option<ProjectWeb>,
    _task: Task<()>,
}

impl Default for WebLink {
    fn default() -> Self {
        Self {
            project: None,
            _task: Task::ready(()),
        }
    }
}

struct MenuTarget {
    path: SharedString,
    file: Option<usize>,
    status: FileStatus,
}

impl ReviewView {
    pub(super) fn refresh_web(&mut self, cx: &mut Context<Self>) {
        match (&self.source, &self.mr) {
            (_, Some(ctx)) => {
                self.web = WebLink {
                    project: project_web_from_mr(&ctx.mr.web_url),
                    _task: Task::ready(()),
                };
            }
            (ReviewSource::Branches { .. }, None) => {
                let repo = self.repo.clone();
                let task = cx.spawn(async move |this, cx| {
                    let remotes = cx
                        .background_spawn(async move { repo.remotes() })
                        .await
                        .unwrap_or_else(|err| {
                            log::warn!("remotes for web links: {err}");
                            Vec::new()
                        });
                    let project = project_web_from_remotes(&remotes);
                    let _ = this.update(cx, |this, _| this.web.project = project);
                });
                self.web._task = task;
            }
            (ReviewSource::WorkingTree | ReviewSource::MergeRequest { .. }, None) => {
                self.web = WebLink::default();
            }
        }
    }

    fn file_line(&self, file: usize, cx: &App) -> u32 {
        let Some(shown) = self.review.as_ref().and_then(|r| r.files.get(file)) else {
            return 1;
        };
        let is_shown = self.current_file == Some(file);
        let changes = self.shown_changes(file);
        let cursor = self
            .change_cursor
            .filter(|_| is_shown)
            .and_then(|k| changes.get(k));
        let selection = is_shown
            .then(|| self.diff.read(cx).selected_lines())
            .flatten()
            .filter(|range| range.path() == &shown.path && range.end_side() == DiffSide::Modified)
            .map(|range| range.end() as u32);
        pick_line(cursor, self.change_scrolled, selection, changes.first())
    }

    fn web_url(&self, file: usize, cx: &App) -> Option<String> {
        let project = self.web.project.as_ref()?;
        let review = self.review.as_ref()?;
        let shown = review.files.get(file)?;
        if self.source == ReviewSource::WorkingTree || review.head_sha.is_empty() {
            return None;
        }
        Some(if shown.status == FileStatus::Deleted {
            blob_url(project, &review.base_sha, &shown.old_path, None)
        } else {
            blob_url(
                project,
                &review.head_sha,
                &shown.new_path,
                Some(self.file_line(file, cx)),
            )
        })
    }

    pub(super) fn copy_shown_path(&self, cx: &mut App) {
        let Some(path) = self
            .current_file
            .and_then(|f| self.review.as_ref()?.files.get(f))
            .map(|f| f.path.to_string())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path));
    }

    pub(super) fn open_shown_in_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.current_file {
            self.open_in_editor(file, cx);
        }
    }

    fn open_in_editor(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(shown) = self.review.as_ref().and_then(|r| r.files.get(file)) else {
            return;
        };
        let relative = shown.new_path.to_string();
        let path: PathBuf = self.repo.root().join(&relative);
        let line = self.file_line(file, cx);
        let template = settings(cx).editor_command.clone();
        let dir = self.repo.root().to_path_buf();
        let executor = cx.background_executor().clone();
        let launch = cx.background_spawn(async move {
            if !path.exists() {
                return (None, Launch::Failed(tr().file_not_on_disk(&relative)));
            }
            let shell = std::env::var("SHELL").ok();
            let invocation = editor_invocation(
                &template,
                &dir.to_string_lossy(),
                &path.to_string_lossy(),
                line,
                HostOs::current(),
                shell.as_deref(),
            );
            let shown = invocation.display();
            let check_exit = invocation.check_exit;
            log::info!("open in editor: {shown}");
            let mut child = match spawn(&invocation, &dir) {
                Ok(child) => child,
                Err(err) => return (Some(shown), Launch::Failed(err.to_string())),
            };
            let started = Instant::now();
            while started.elapsed() < FAILURE_WINDOW {
                match child.try_wait() {
                    Ok(Some(_)) if !check_exit => return (Some(shown), Launch::Done),
                    Ok(Some(status)) => return (Some(shown), Launch::Exited(status)),
                    Ok(None) => executor.timer(POLL_FAST).await,
                    Err(err) => return (Some(shown), Launch::Failed(err.to_string())),
                }
            }
            (Some(shown), Launch::Running(child))
        });
        cx.spawn(async move |this, cx| {
            let (shown, launch) = launch.await;
            let failure = match launch {
                Launch::Failed(message) => Some(message),
                Launch::Done => None,
                Launch::Exited(status) if status.success() => None,
                Launch::Exited(status) => Some(status.to_string()),
                Launch::Running(child) => {
                    cx.background_spawn(reap(child, cx.background_executor().clone()))
                        .detach();
                    None
                }
            };
            let Some(message) = failure else {
                return;
            };
            let message = match shown {
                Some(shown) => format!("{shown}: {message}"),
                None => message,
            };
            log::warn!("open in editor: {message}");
            let _ = this.update(cx, |this, cx| {
                this.notify(
                    Notification::error(message).title(tr().editor_failed.clone()),
                    cx,
                );
            });
        })
        .detach();
    }

    fn menu_target(&self, node: usize) -> Option<MenuTarget> {
        let node = self.tree_node(node)?;
        let file = node.file();
        let shown = file.and_then(|f| self.review.as_ref()?.files.get(f));
        Some(MenuTarget {
            path: shown.map_or_else(|| node.path.clone().into(), |f| f.path.clone()),
            file,
            status: shown.map_or(FileStatus::Modified, |f| f.status),
        })
    }

    fn shown_target(&self) -> Option<MenuTarget> {
        let file = self.current_file?;
        let shown = self.review.as_ref()?.files.get(file)?;
        Some(MenuTarget {
            path: shown.path.clone(),
            file: Some(file),
            status: shown.status,
        })
    }

    fn web_label(&self) -> Option<SharedString> {
        let project = self.web.project.as_ref()?;
        if self.source == ReviewSource::WorkingTree {
            return None;
        }
        Some(match project.forge {
            Forge::GitLab => tr().open_in_gitlab.clone(),
            Forge::GitHub => tr().open_on_github.clone(),
        })
    }
}

async fn reap(mut child: Child, executor: gpui::BackgroundExecutor) {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                log::debug!("editor command exited later: {status}");
                return;
            }
            Ok(None) => executor.timer(POLL_SLOW).await,
            Err(err) => {
                log::warn!("waiting for the editor command: {err}");
                return;
            }
        }
    }
}

fn file_items(
    menu: PopupMenu,
    view: &Entity<ReviewView>,
    target: MenuTarget,
    with_keys: bool,
    cx: &App,
) -> PopupMenu {
    let s = tr();
    let path = target.path.clone();
    let copy = PopupMenuItem::new(s.copy_path.clone()).on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()))
    });
    let mut menu = menu.item(if with_keys {
        copy.action(Box::new(CopyPath))
    } else {
        copy
    });
    let Some(file) = target.file else {
        return menu;
    };
    if target.status != FileStatus::Deleted {
        let editor_view = view.clone();
        let open = PopupMenuItem::new(s.open_in_editor.clone()).on_click(move |_, _, cx| {
            editor_view.update(cx, |this, cx| this.open_in_editor(file, cx))
        });
        menu = menu.item(if with_keys {
            open.action(Box::new(OpenInEditor))
        } else {
            open
        });
    }
    if let Some(label) = view.read(cx).web_label() {
        let browser_view = view.clone();
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
            if let Some(url) = browser_view.read(cx).web_url(file, cx) {
                cx.open_url(&url);
            }
        }));
    }
    menu
}

pub(super) fn row_menu(
    menu: PopupMenu,
    view: &Entity<ReviewView>,
    node: usize,
    cx: &App,
) -> PopupMenu {
    match view.read(cx).menu_target(node) {
        Some(target) => file_items(menu, view, target, false, cx).separator(),
        None => menu,
    }
}

pub(super) fn shown_file_menu(menu: PopupMenu, view: &Entity<ReviewView>, cx: &App) -> PopupMenu {
    match view.read(cx).shown_target() {
        Some(target) => file_items(menu, view, target, true, cx),
        None => menu,
    }
}

#[cfg(test)]
#[path = "file_actions_tests.rs"]
mod tests;
