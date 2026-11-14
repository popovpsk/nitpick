use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

use gpui::{
    Action, App, Global, KeyBinding, KeyBindingContextPredicate, Keystroke, OperatingSystem,
    SharedString,
};
use ui_resources::i18n::Strings;

use crate::review::{
    CollapseSelected, ExpandSelected, FocusDiff, FocusTree, NextChange, NextFile, OpenSelected,
    PreviousChange, PreviousFile, Refresh, SelectNext, SelectPrevious, ToggleBlame, ToggleTree,
    ToggleViewed,
};
use crate::settings_store::{SettingsStore, settings, update_settings};
use crate::workspace::{
    AddRepositories, CloseWindow, NewReview, NewWindow, OpenRepository, ShowNextWindowTab,
    ShowPreviousWindowTab, ShowSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Review,
    Tree,
    Window,
}

pub struct Command {
    pub id: &'static str,
    pub group: Group,
    pub context: Option<&'static str>,
    pub defaults: &'static [&'static str],
    pub action: fn() -> Box<dyn Action>,
    pub title: fn(&Strings) -> SharedString,
}

const REVIEW: Option<&str> = Some("Review");
const TREE: Option<&str> = Some("ReviewTree");

pub static COMMANDS: &[Command] = &[
    Command {
        id: "review.next_change",
        group: Group::Review,
        context: REVIEW,
        defaults: &["alt-down", "f7"],
        action: || Box::new(NextChange),
        title: |s| s.next_change.clone(),
    },
    Command {
        id: "review.previous_change",
        group: Group::Review,
        context: REVIEW,
        defaults: &["alt-up", "shift-f7"],
        action: || Box::new(PreviousChange),
        title: |s| s.previous_change.clone(),
    },
    Command {
        id: "review.next_file",
        group: Group::Review,
        context: REVIEW,
        defaults: &["alt-right"],
        action: || Box::new(NextFile),
        title: |s| s.cmd_next_file.clone(),
    },
    Command {
        id: "review.previous_file",
        group: Group::Review,
        context: REVIEW,
        defaults: &["alt-left"],
        action: || Box::new(PreviousFile),
        title: |s| s.cmd_previous_file.clone(),
    },
    Command {
        id: "review.toggle_viewed",
        group: Group::Review,
        context: REVIEW,
        defaults: &["space"],
        action: || Box::new(ToggleViewed),
        title: |s| s.cmd_toggle_viewed.clone(),
    },
    Command {
        id: "review.toggle_tree",
        group: Group::Review,
        context: REVIEW,
        defaults: &["secondary-b"],
        action: || Box::new(ToggleTree),
        title: |s| s.toggle_tree.clone(),
    },
    Command {
        id: "review.focus_tree",
        group: Group::Review,
        context: REVIEW,
        defaults: &["secondary-1"],
        action: || Box::new(FocusTree),
        title: |s| s.cmd_focus_tree.clone(),
    },
    Command {
        id: "review.focus_diff",
        group: Group::Review,
        context: REVIEW,
        defaults: &["secondary-2"],
        action: || Box::new(FocusDiff),
        title: |s| s.cmd_focus_diff.clone(),
    },
    Command {
        id: "review.refresh",
        group: Group::Review,
        context: REVIEW,
        defaults: &["secondary-r"],
        action: || Box::new(Refresh),
        title: |s| s.refresh.clone(),
    },
    Command {
        id: "review.toggle_blame",
        group: Group::Review,
        context: REVIEW,
        defaults: &[],
        action: || Box::new(ToggleBlame),
        title: |s| s.cmd_toggle_blame.clone(),
    },
    Command {
        id: "tree.select_next",
        group: Group::Tree,
        context: TREE,
        defaults: &["down", "j"],
        action: || Box::new(SelectNext),
        title: |s| s.cmd_select_next.clone(),
    },
    Command {
        id: "tree.select_previous",
        group: Group::Tree,
        context: TREE,
        defaults: &["up", "k"],
        action: || Box::new(SelectPrevious),
        title: |s| s.cmd_select_previous.clone(),
    },
    Command {
        id: "tree.collapse",
        group: Group::Tree,
        context: TREE,
        defaults: &["left", "h"],
        action: || Box::new(CollapseSelected),
        title: |s| s.cmd_collapse.clone(),
    },
    Command {
        id: "tree.expand",
        group: Group::Tree,
        context: TREE,
        defaults: &["right", "l"],
        action: || Box::new(ExpandSelected),
        title: |s| s.cmd_expand.clone(),
    },
    Command {
        id: "tree.open",
        group: Group::Tree,
        context: TREE,
        defaults: &["enter"],
        action: || Box::new(OpenSelected),
        title: |s| s.cmd_open_selected.clone(),
    },
    Command {
        id: "tree.toggle_viewed",
        group: Group::Tree,
        context: TREE,
        defaults: &["space", "v"],
        action: || Box::new(ToggleViewed),
        title: |s| s.cmd_tree_toggle_viewed.clone(),
    },
    Command {
        id: "tree.next_file",
        group: Group::Tree,
        context: TREE,
        defaults: &["n"],
        action: || Box::new(NextFile),
        title: |s| s.cmd_next_file.clone(),
    },
    Command {
        id: "tree.previous_file",
        group: Group::Tree,
        context: TREE,
        defaults: &["p"],
        action: || Box::new(PreviousFile),
        title: |s| s.cmd_previous_file.clone(),
    },
    Command {
        id: "window.open_repository",
        group: Group::Window,
        context: None,
        defaults: &["secondary-o"],
        action: || Box::new(OpenRepository),
        title: |s| s.open_repository.clone(),
    },
    Command {
        id: "window.add_repositories",
        group: Group::Window,
        context: None,
        defaults: &[],
        action: || Box::new(AddRepositories),
        title: |s| s.cmd_add_repositories.clone(),
    },
    Command {
        id: "window.new_review",
        group: Group::Window,
        context: None,
        defaults: &["secondary-n"],
        action: || Box::new(NewReview),
        title: |s| s.new_review.clone(),
    },
    Command {
        id: "window.settings",
        group: Group::Window,
        context: None,
        defaults: &["secondary-,"],
        action: || Box::new(ShowSettings),
        title: |s| s.settings.clone(),
    },
    Command {
        id: "window.new_window",
        group: Group::Window,
        context: None,
        defaults: &["secondary-shift-n"],
        action: || Box::new(NewWindow),
        title: |s| s.new_window.clone(),
    },
    Command {
        id: "window.close_window",
        group: Group::Window,
        context: None,
        defaults: &["secondary-w"],
        action: || Box::new(CloseWindow),
        title: |s| s.close_window.clone(),
    },
    Command {
        id: "window.next_tab",
        group: Group::Window,
        context: None,
        defaults: &["secondary-}"],
        action: || Box::new(ShowNextWindowTab),
        title: |s| s.cmd_next_tab.clone(),
    },
    Command {
        id: "window.previous_tab",
        group: Group::Window,
        context: None,
        defaults: &["secondary-{"],
        action: || Box::new(ShowPreviousWindowTab),
        title: |s| s.cmd_previous_tab.clone(),
    },
];

struct BaseKeymap(Vec<KeyBinding>);

impl Global for BaseKeymap {}

#[derive(Default)]
struct AppliedOverrides(BTreeMap<String, Vec<String>>);

impl Global for AppliedOverrides {}

pub fn init(cx: &mut App) {
    let base: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.set_global(BaseKeymap(base));
    apply(cx);
    cx.observe_global::<SettingsStore>(|cx| {
        if cx.global::<AppliedOverrides>().0 != settings(cx).keybindings {
            apply(cx);
        }
    })
    .detach();
}

fn apply(cx: &mut App) {
    let mut bindings = cx.global::<BaseKeymap>().0.clone();
    for command in COMMANDS {
        let predicate = command
            .context
            .and_then(|context| KeyBindingContextPredicate::parse(context).ok())
            .map(Rc::new);
        for key in keys(command, cx) {
            match KeyBinding::load(&key, (command.action)(), predicate.clone(), false, None, cx) {
                Ok(binding) => bindings.push(binding),
                Err(err) => log::warn!("shortcut {key:?} for {}: {err}", command.id),
            }
        }
    }
    cx.set_global(AppliedOverrides(settings(cx).keybindings.clone()));
    log::info!(
        "keymap: {} bindings, {} customized commands",
        bindings.len(),
        settings(cx).keybindings.len()
    );
    cx.clear_key_bindings();
    cx.bind_keys(bindings);
    crate::app_menu::set_menus(cx);
}

pub fn command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

pub fn normalize(key: &str, os: OperatingSystem) -> Option<String> {
    Keystroke::parse_with_os(key, os)
        .ok()
        .map(|keystroke| keystroke.unparse(os))
}

pub fn default_keys(command: &Command, os: OperatingSystem) -> Vec<String> {
    command
        .defaults
        .iter()
        .filter_map(|key| normalize(key, os))
        .collect()
}

fn cached_defaults(command: &Command, cx: &App) -> Vec<String> {
    static CACHE: OnceLock<Vec<Vec<String>>> = OnceLock::new();
    let os = cx.platform_info().os;
    let all = CACHE.get_or_init(|| COMMANDS.iter().map(|c| default_keys(c, os)).collect());
    COMMANDS
        .iter()
        .position(|c| c.id == command.id)
        .and_then(|ix| all.get(ix).cloned())
        .unwrap_or_default()
}

pub fn keys(command: &Command, cx: &App) -> Vec<String> {
    match settings(cx).keybindings.get(command.id) {
        Some(keys) => keys.clone(),
        None => cached_defaults(command, cx),
    }
}

pub fn is_customized(command: &Command, cx: &App) -> bool {
    settings(cx).keybindings.contains_key(command.id)
}

fn overlaps(a: Option<&str>, b: Option<&str>) -> bool {
    a.is_none() || b.is_none() || a == b
}

pub fn conflicts(key: &str, command: &Command, cx: &App) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|other| {
            other.id != command.id
                && overlaps(other.context, command.context)
                && keys(other, cx).iter().any(|k| k == key)
        })
        .collect()
}

pub fn builtin_conflict(key: &str, cx: &App) -> bool {
    let Ok(keystroke) = Keystroke::parse_with_os(key, cx.platform_info().os) else {
        return false;
    };
    cx.global::<BaseKeymap>().0.iter().any(|binding| {
        matches!(binding.keystrokes(), [only]
            if only.inner().modifiers == keystroke.modifiers && only.inner().key == keystroke.key)
    })
}

pub fn needs_modifier(command: &Command, keystroke: &Keystroke) -> bool {
    let function_key = keystroke
        .key
        .strip_prefix('f')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    command.group == Group::Window
        && !keystroke.modifiers.control
        && !keystroke.modifiers.platform
        && !function_key
}

fn release(
    key: &str,
    command: &Command,
    changes: &mut Vec<(&'static Command, Vec<String>)>,
    cx: &App,
) {
    for other in conflicts(key, command, cx) {
        let base = changes
            .iter()
            .find(|(c, _)| c.id == other.id)
            .map(|(_, list)| list.clone())
            .unwrap_or_else(|| keys(other, cx));
        let rest: Vec<String> = base.into_iter().filter(|k| k != key).collect();
        changes.retain(|(c, _)| c.id != other.id);
        changes.push((other, rest));
    }
}

fn store(changes: Vec<(&'static Command, Vec<String>)>, cx: &mut App) {
    let defaults: Vec<(&'static Command, Vec<String>)> = changes
        .iter()
        .map(|(command, _)| (*command, cached_defaults(command, cx)))
        .collect();
    update_settings(cx, move |s| {
        for ((command, list), (_, default)) in changes.into_iter().zip(defaults) {
            if list == default {
                s.keybindings.remove(command.id);
            } else {
                s.keybindings.insert(command.id.to_string(), list);
            }
        }
    });
}

pub fn assign(
    command: &'static Command,
    original: Option<String>,
    key: Option<String>,
    cx: &mut App,
) {
    let current = keys(command, cx);
    let mut own = current.clone();
    if let Some(original) = &original {
        own.retain(|k| k != original);
    }
    let mut changes = Vec::new();
    if let Some(key) = &key {
        release(key, command, &mut changes, cx);
        if !own.contains(key) {
            let at = original
                .as_ref()
                .and_then(|o| current.iter().position(|k| k == o))
                .unwrap_or(own.len())
                .min(own.len());
            own.insert(at, key.clone());
        }
    }
    changes.push((command, own));
    store(changes, cx);
}

pub fn reset(command: Option<&'static Command>, cx: &mut App) {
    match command {
        Some(command) => {
            let defaults = cached_defaults(command, cx);
            let mut changes = Vec::new();
            for key in &defaults {
                release(key, command, &mut changes, cx);
            }
            changes.push((command, defaults));
            store(changes, cx);
        }
        None => update_settings(cx, |s| s.keybindings.clear()),
    }
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod tests;
