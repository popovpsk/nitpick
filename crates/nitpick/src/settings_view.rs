use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use agent::Model;
use forge::ForgeKind;
use gpui::{
    Anchor, AnyElement, App, AppContext as _, Axis, Context, ElementId, Entity, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, actions, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, ThemeRegistry, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    searchable_list::SearchableVec,
    select::{Select, SelectEvent, SelectState},
    setting::{SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use ui_resources::Icons;
use ui_resources::i18n::{Lang, Strings};
use ui_resources::themes::ZedThemeFile;
use updater::UpdateState;

use crate::agent_models::{self, Choice, ModelCache};
use crate::agent_profiles::{Profile, format_custom_agents, parse_custom_agents, set_override};
use crate::keymap::{self, COMMANDS, Command, Group};
use crate::localization::{sync_active_lang, tr};
use crate::review::{HostOs, editor_presets};
use crate::settings_store::{
    CustomAgent, LangChoice, SettingsStore, ThemeChoice, TreeSide, UI_SCALES, WrapLines,
    apply_saved_theme, apply_theme, end_syntax_preview, preview_syntax_colors, repo_key, settings,
    syntax_palette_names, update_settings,
};
use crate::shortcut_dialog::{self, keystroke_element, where_label};
use crate::workspace::{CloseWindow, ShowSettings};
use crate::zoom::{ZoomIn, ZoomOut, ZoomReset};

actions!(settings_view, [CloseSettings]);

const FIELD_MAX_WIDTH: f32 = 440.;
const HIDDEN_SAVE_DELAY: Duration = Duration::from_millis(400);
const HIDDEN_MIN_ROWS: usize = 10;
const HIDDEN_MAX_ROWS: usize = 30;
const HIDDEN_FIELD_WIDTH: f32 = 440.;
const HIDDEN_EXAMPLE: &str = "package-lock.json\n*.min.js\n*.pb.go\nmocks/\n__snapshots__/\ndist/";
const EDITOR_SAVE_DELAY: Duration = Duration::from_millis(400);
const AGENT_SAVE_DELAY: Duration = Duration::from_millis(400);
const AGENT_CUSTOM_MIN_ROWS: usize = 3;
const AGENT_CUSTOM_MAX_ROWS: usize = 10;
const AGENT_INSTRUCTIONS_MIN_ROWS: usize = 3;
const AGENT_INSTRUCTIONS_MAX_ROWS: usize = 12;
const AGENT_CUSTOM_EXAMPLE: &str = "my-agent = my-agent acp";

fn invalid_patterns(patterns: &[String]) -> Option<SharedString> {
    let (_, errors) = review_core::HideRules::new(patterns, &[]);
    (!errors.is_empty()).then(|| {
        let list: Vec<&str> = errors.iter().map(|e| e.pattern.as_str()).collect();
        tr().hidden_files_invalid(&list.join(", ")).into()
    })
}

fn shortcut_group(title: SharedString, group: Group, s: &Strings) -> SettingGroup {
    let description: SharedString = match group {
        Group::Review => format!("{} · {}", where_label(group, s), s.shortcuts_desc).into(),
        _ => where_label(group, s),
    };
    let mut items = SettingGroup::new().title(title).description(description);
    for (ix, command) in COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, c)| c.group == group)
    {
        items = items.item(SettingItem::new(
            (command.title)(s),
            SettingField::render(move |_, _, cx| shortcut_keys(command, ix as u64, cx)),
        ));
    }
    if group == Group::Window {
        items = items.item(SettingItem::render(|_, _, _| {
            h_flex().w_full().justify_end().child(
                Button::new("reset-all-shortcuts")
                    .ghost()
                    .small()
                    .icon(Icon::new(Icons::Undo2))
                    .label(tr().reset_all_shortcuts.clone())
                    .on_click(|_, _, cx| keymap::reset(None, cx)),
            )
        }));
    }
    items
}

fn shortcut_keys(command: &'static Command, ix: u64, cx: &App) -> AnyElement {
    let s = tr();
    let theme = cx.theme();
    let hover = theme.list_hover;
    let customized = keymap::is_customized(command, cx);
    h_flex()
        .gap_1()
        .items_center()
        .justify_end()
        .children(
            keymap::keys(command, cx)
                .into_iter()
                .enumerate()
                .map(|(ix, key)| {
                    div()
                        .id(ElementId::NamedInteger(command.id.into(), ix as u64))
                        .px_0p5()
                        .py_0p5()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(keystroke_element(&key, cx))
                        .on_click(move |_, window, cx| {
                            shortcut_dialog::open(command, Some(key.clone()), window, cx)
                        })
                }),
        )
        .child(
            Button::new(ElementId::NamedInteger("shortcut-add".into(), ix))
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip(s.add_shortcut.clone())
                .on_click(move |_, window, cx| shortcut_dialog::open(command, None, window, cx)),
        )
        .when(customized, |el| {
            el.child(
                Button::new(ElementId::NamedInteger("shortcut-reset".into(), ix))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Icons::Undo2))
                    .tooltip(s.reset_shortcut.clone())
                    .on_click(move |_, _, cx| keymap::reset(Some(command), cx)),
            )
        })
        .into_any_element()
}

fn hidden_rules_item(
    title: SharedString,
    description: SharedString,
    rules: &HiddenRules,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .gap_1()
        .child(div().text_sm().child(title))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(description),
        )
        .child(
            div()
                .relative()
                .w_full()
                .max_w(px(HIDDEN_FIELD_WIDTH))
                .child(Textarea::new(&rules.input).font_family(theme.mono_font_family.clone()))
                .when(rules.empty, |field| {
                    field.child(
                        div()
                            .absolute()
                            .top(px(9.))
                            .left(px(11.))
                            .text_sm()
                            .line_height(rems(1.25))
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground.opacity(0.45))
                            .child(HIDDEN_EXAMPLE),
                    )
                }),
        )
        .children(
            rules
                .invalid
                .clone()
                .map(|invalid| div().text_xs().text_color(theme.danger).child(invalid)),
        )
        .into_any_element()
}

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CloseSettings,
        Some("SettingsScreen"),
    )]);
}

#[derive(Clone)]
enum ForgeStatus {
    Idle,
    Checking,
    Ok(SharedString),
    Failed(SharedString),
}

struct ForgeForm {
    url: Entity<InputState>,
    token: Entity<InputState>,
    status: ForgeStatus,
    _check: Task<()>,
}

impl ForgeForm {
    fn new(
        url: &str,
        url_placeholder: &'static str,
        token: &str,
        token_placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> Self {
        let url = url.to_string();
        let token = token.to_string();
        Self {
            url: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(url_placeholder)
                    .default_value(url)
            }),
            token: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(token_placeholder)
                    .masked(true)
                    .default_value(token)
            }),
            status: ForgeStatus::Idle,
            _check: Task::ready(()),
        }
    }
}

struct AgentForm {
    profile: Profile,
    profile_name: SharedString,
    command: Entity<InputState>,
    model: Entity<InputState>,
    command_baseline: String,
    model_baseline: String,
    custom: Entity<TextareaState>,
    custom_baseline: Vec<CustomAgent>,
    custom_invalid: Option<SharedString>,
    custom_empty: bool,
    instructions: Entity<TextareaState>,
    instructions_baseline: String,
    generation: Rc<Cell<u64>>,
    custom_generation: Rc<Cell<u64>>,
    instructions_generation: Rc<Cell<u64>>,
    status: ForgeStatus,
    model_select: Entity<SelectState<SearchableVec<ModelChoice>>>,
    fetched: Vec<Model>,
    catalog: Catalog,
    catalog_key: Option<(agent::HarnessKind, String)>,
    catalog_generation: u64,
    _models: Task<()>,
    _check: Task<()>,
    _subscriptions: [Subscription; 5],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Catalog {
    Loading,
    Ready,
    Failed,
}

#[derive(Clone)]
struct ModelChoice(Choice);

impl gpui_component::searchable_list::SearchableListItem for ModelChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.0.title.clone().into()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .min_w_0()
            .child(div().child(SharedString::from(self.0.title.clone())))
            .children(self.0.description.clone().map(|text| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(SharedString::from(text))
            }))
    }

    fn value(&self) -> &Self::Value {
        &self.0.value
    }
}

fn model_choices(models: &[Model], stored: &str, s: &Strings) -> Vec<ModelChoice> {
    agent_models::choices(
        models,
        stored,
        &s.assistant_model_default,
        &s.assistant_model_custom,
    )
    .into_iter()
    .map(ModelChoice)
    .collect()
}

fn stored<'a>(map: &'a BTreeMap<String, String>, id: &str) -> &'a str {
    map.get(id).map_or("", |value| value.trim())
}

impl AgentForm {
    fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let current = settings(cx).agent.clone();
        let profile = current.active_profile();
        let command_text = stored(&current.commands, &profile.id).to_string();
        let model_text = stored(&current.models, &profile.id).to_string();
        let command = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(profile.default_command.clone())
                .default_value(command_text.clone())
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr().assistant_model_placeholder.clone())
                .default_value(model_text.clone())
        });
        let custom_text = format_custom_agents(&current.custom);
        let custom_empty = custom_text.is_empty();
        let custom = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(AGENT_CUSTOM_MIN_ROWS, AGENT_CUSTOM_MAX_ROWS)
                .default_value(custom_text)
        });
        let instructions_text = current.instructions.trim().to_string();
        let instructions = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(AGENT_INSTRUCTIONS_MIN_ROWS, AGENT_INSTRUCTIONS_MAX_ROWS)
                .placeholder(tr().assistant_instructions_placeholder.clone())
                .default_value(instructions_text.clone())
        });
        let on_fields = |this: &mut SettingsView,
                         _: &Entity<InputState>,
                         event: &InputEvent,
                         window: &mut Window,
                         cx: &mut Context<SettingsView>| {
            if matches!(event, InputEvent::Change) {
                this.schedule_agent_save(window, cx);
            }
        };
        let model_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(model_choices(&[], &model_text, tr())),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let subscriptions = [
            cx.subscribe_in(&command, window, on_fields),
            cx.subscribe_in(&model, window, on_fields),
            cx.subscribe_in(
                &model_select,
                window,
                |this, _, event: &SelectEvent<SearchableVec<ModelChoice>>, window, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    let value = value.clone();
                    this.agent
                        .model
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                },
            ),
            cx.subscribe(&custom, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_custom_save(cx);
                }
            }),
            cx.subscribe(&instructions, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_instructions_save(cx);
                }
            }),
        ];
        Self {
            profile_name: profile.name.clone().into(),
            profile,
            command,
            model,
            command_baseline: command_text,
            model_baseline: model_text,
            custom,
            custom_baseline: current.custom,
            custom_invalid: None,
            custom_empty,
            instructions,
            instructions_baseline: instructions_text,
            generation: Rc::default(),
            custom_generation: Rc::default(),
            instructions_generation: Rc::default(),
            status: ForgeStatus::Idle,
            model_select,
            fetched: Vec::new(),
            catalog: Catalog::Loading,
            catalog_key: None,
            catalog_generation: 0,
            _models: Task::ready(()),
            _check: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }
}

struct HiddenRules {
    input: Entity<TextareaState>,
    invalid: Option<SharedString>,
    empty: bool,
    baseline: Vec<String>,
    generation: Rc<Cell<u64>>,
}

impl HiddenRules {
    fn new(input: Entity<TextareaState>) -> Self {
        Self {
            input,
            invalid: None,
            empty: true,
            baseline: Vec::new(),
            generation: Rc::default(),
        }
    }

    fn load(&mut self, list: &[String], force: bool, window: &mut Window, cx: &mut App) {
        let focused = self
            .input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if !force && (focused || self.baseline == list) {
            return;
        }
        self.baseline = list.to_vec();
        self.invalid = invalid_patterns(list);
        let text = list.join("\n");
        self.empty = text.is_empty();
        self.input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }
}

struct RepoRules {
    root: PathBuf,
    name: SharedString,
    path: SharedString,
    rules: HiddenRules,
    _subscription: Subscription,
}

impl RepoRules {
    fn new(root: PathBuf, window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let input =
            cx.new(|cx| TextareaState::new(window, cx).auto_grow(HIDDEN_MIN_ROWS, HIDDEN_MAX_ROWS));
        let save_root = root.clone();
        let subscription = cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_hidden_save(Some(save_root.clone()), cx);
            }
        });
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        Self {
            path: root.to_string_lossy().into_owned().into(),
            root,
            name: name.into(),
            rules: HiddenRules::new(input),
            _subscription: subscription,
        }
    }
}

fn hidden_roots(
    opener: Option<&Path>,
    open: &[PathBuf],
    with_rules: impl Iterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    let mut rest: Vec<PathBuf> = open.iter().cloned().chain(with_rules).collect();
    rest.sort_by_key(|root| {
        root.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    let mut roots: Vec<PathBuf> = opener.map(Path::to_path_buf).into_iter().collect();
    for root in rest {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    HiddenFiles,
    Assistant,
}

const HIDDEN_FILES_PAGE: usize = 1;
const ASSISTANT_PAGE: usize = 3;

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub struct SettingsView {
    focus: FocusHandle,
    generation: u64,
    revealed: Option<SelectIndex>,
    gitlab: ForgeForm,
    github: ForgeForm,
    editor_command: Entity<InputState>,
    editor_generation: Rc<Cell<u64>>,
    editor_baseline: String,
    _editor_subscription: Subscription,
    agent: AgentForm,
    hidden_global: HiddenRules,
    repo_rules: Vec<RepoRules>,
    repo: Option<(PathBuf, SharedString)>,
    _hidden_subscription: Subscription,
    _settings_sync: Option<Subscription>,
    zed_files: Rc<Vec<ZedThemeFile>>,
    _zed_discovery: Task<()>,
    _zed_import: Task<()>,
    _updater: Option<Subscription>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = settings(cx).clone();
        let gitlab = ForgeForm::new(
            &current.gitlab_url,
            "https://gitlab.example.com",
            &current.gitlab_token,
            "glpat-…",
            window,
            cx,
        );
        let github = ForgeForm::new(
            &current.github_url,
            forge::GITHUB_DEFAULT_URL,
            &current.github_token,
            "github_pat_…",
            window,
            cx,
        );
        let editor_command = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("code {root} -g {file}:{line}")
                .default_value(current.editor_command.clone())
        });
        let editor_subscription =
            cx.subscribe(&editor_command, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_editor_save(cx);
                }
            });
        let agent = AgentForm::new(window, cx);
        let zed_discovery = cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async { ui_resources::themes::zed_theme_files() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.zed_files = Rc::new(files);
                cx.notify();
            });
        });
        let hidden_input = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| TextareaState::new(window, cx).auto_grow(HIDDEN_MIN_ROWS, HIDDEN_MAX_ROWS))
        };
        let global_input = hidden_input(window, cx);
        let global_subscription = cx.subscribe(&global_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_hidden_save(None, cx);
            }
        });
        let mut this = Self {
            focus: cx.focus_handle(),
            generation: 0,
            revealed: None,
            gitlab,
            github,
            editor_command,
            editor_generation: Rc::default(),
            editor_baseline: current.editor_command.clone(),
            _editor_subscription: editor_subscription,
            agent,
            hidden_global: HiddenRules::new(global_input),
            repo_rules: Vec::new(),
            repo: None,
            _hidden_subscription: global_subscription,
            _settings_sync: None,
            zed_files: Rc::default(),
            _zed_discovery: zed_discovery,
            _zed_import: Task::ready(()),
            _updater: updater::handle(cx)
                .map(|updater| cx.observe(&updater, |_, _, cx| cx.notify())),
        };
        this.reload_hidden(true, window, cx);
        this.refresh_models(false, window, cx);
        this._settings_sync = Some(cx.observe_global_in::<SettingsStore>(
            window,
            |this, window, cx| {
                this.reload_hidden(false, window, cx);
                this.reload_editor_command(window, cx);
                this.reload_agent(window, cx);
                cx.notify();
            },
        ));
        this
    }

    fn keep_focus_inside(&self, window: &mut Window, cx: &mut Context<Self>) {
        let stray = |window: &mut Window, cx: &mut App| {
            window.is_window_active()
                && window.focused(cx).is_none()
                && !window.has_active_dialog(cx)
                && !window.has_active_sheet(cx)
        };
        if stray(window, cx) {
            let focus = self.focus.clone();
            cx.defer_in(window, move |_, window, cx| {
                if stray(window, cx) {
                    window.focus(&focus, cx);
                }
            });
        }
    }

    pub fn reveal(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        let page_ix = match section {
            SettingsSection::HiddenFiles => HIDDEN_FILES_PAGE,
            SettingsSection::Assistant => ASSISTANT_PAGE,
        };
        self.generation += 1;
        self.revealed = Some(SelectIndex {
            page_ix,
            group_ix: None,
        });
        cx.notify();
    }

    pub fn set_repository(
        &mut self,
        root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.repo = root.map(|root| {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string_lossy().into_owned());
            (root, name.into())
        });
        self.reload_hidden(true, window, cx);
        cx.notify();
    }

    fn reload_hidden(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = settings(cx).clone();
        self.hidden_global
            .load(&current.hidden_files, force, window, cx);
        let roots = hidden_roots(
            self.repo.as_ref().map(|(root, _)| root.as_path()),
            &crate::window_registry::open_repos(cx),
            current.repo_hidden_files.keys().map(PathBuf::from),
        );
        let mut kept: Vec<RepoRules> = Vec::with_capacity(roots.len());
        for root in roots {
            let entry = match self.repo_rules.iter().position(|r| r.root == root) {
                Some(ix) => self.repo_rules.remove(ix),
                None => RepoRules::new(root, window, cx),
            };
            kept.push(entry);
        }
        if !force {
            kept.append(&mut self.repo_rules);
        }
        self.repo_rules = kept;
        for entry in &mut self.repo_rules {
            let list = current.repo_hidden(&entry.root).to_vec();
            entry.rules.load(&list, force, window, cx);
        }
    }

    fn reload_editor_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = settings(cx).editor_command.clone();
        let focused = self
            .editor_command
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if focused || self.editor_baseline == saved {
            return;
        }
        self.editor_baseline = saved.clone();
        self.editor_command
            .update(cx, |state, cx| state.set_value(saved, window, cx));
    }

    fn schedule_editor_save(&mut self, cx: &mut Context<Self>) {
        let typed = self.editor_command.read(cx).value().trim().to_string();
        let generation = self.editor_generation.get() + 1;
        self.editor_generation.set(generation);
        let counter = self.editor_generation.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EDITOR_SAVE_DELAY).await;
            if counter.get() != generation {
                return;
            }
            let baseline = typed.clone();
            let _ = this.update(cx, |this, _| this.editor_baseline = baseline);
            cx.update(|cx| {
                if settings(cx).editor_command != typed {
                    update_settings(cx, move |s| s.editor_command = typed);
                }
            });
        })
        .detach();
    }

    fn schedule_agent_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.agent.generation.get() + 1;
        self.agent.generation.set(generation);
        let counter = self.agent.generation.clone();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AGENT_SAVE_DELAY).await;
            if counter.get() != generation {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.save_agent_fields(window, cx));
        })
        .detach();
    }

    fn save_agent_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.agent.profile.id.clone();
        let command = self.agent.command.read(cx).value().trim().to_string();
        let model = self.agent.model.read(cx).value().trim().to_string();
        let command_changed = self.agent.command_baseline != command;
        self.agent.command_baseline = command.clone();
        self.agent.model_baseline = model.clone();
        let current = &settings(cx).agent;
        if stored(&current.commands, &id) != command || stored(&current.models, &id) != model {
            update_settings(cx, move |s| {
                set_override(&mut s.agent.commands, &id, &command);
                set_override(&mut s.agent.models, &id, &model);
            });
        }
        if command_changed {
            self.refresh_models(false, window, cx);
        } else {
            self.sync_model_select(window, cx);
        }
    }

    fn refresh_models(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.agent.profile.kind;
        let typed = self.agent.command.read(cx).value().to_string();
        let command = agent_models::effective_command(&typed, &self.agent.profile.default_command);
        let key = (kind, command.clone());
        if !force && self.agent.catalog_key.as_ref() == Some(&key) {
            return;
        }
        self.agent.catalog_key = Some(key);
        self.agent.catalog_generation += 1;
        let generation = self.agent.catalog_generation;
        if !force
            && let Some(models) = cx
                .default_global::<ModelCache>()
                .get(kind, &command)
                .cloned()
        {
            self.agent._models = Task::ready(());
            self.install_models(models, window, cx);
            return;
        }
        self.agent.catalog = Catalog::Loading;
        self.agent.fetched.clear();
        self.agent._models = cx.spawn_in(window, async move |this, cx| {
            let result = agent::models(kind, &command).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.agent.catalog_generation != generation {
                    return;
                }
                match result {
                    Ok(models) => {
                        cx.default_global::<ModelCache>()
                            .put(kind, &command, models.clone());
                        this.install_models(models, window, cx);
                    }
                    Err(err) => {
                        log::debug!("the model list of `{command}`: {err}");
                        this.agent.catalog = Catalog::Failed;
                        cx.notify();
                    }
                }
            });
        });
        cx.notify();
    }

    fn install_models(&mut self, models: Vec<Model>, window: &mut Window, cx: &mut Context<Self>) {
        self.agent.catalog = if models.is_empty() {
            Catalog::Failed
        } else {
            Catalog::Ready
        };
        self.agent.fetched = models;
        self.sync_model_select(window, cx);
        cx.notify();
    }

    fn sync_model_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stored = self.agent.model.read(cx).value().trim().to_string();
        let items = model_choices(&self.agent.fetched, &stored, tr());
        let position = agent_models::position(
            &items.iter().map(|item| item.0.clone()).collect::<Vec<_>>(),
            &stored,
        );
        self.agent.model_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(items), window, cx);
            select.set_selected_index(Some(gpui_component::IndexPath::new(position)), window, cx);
        });
    }

    fn schedule_custom_save(&mut self, cx: &mut Context<Self>) {
        let text = self.agent.custom.read(cx).value();
        self.agent.custom_empty = text.is_empty();
        let parsed = parse_custom_agents(&text);
        self.agent.custom_invalid = (!parsed.ignored.is_empty()).then(|| {
            tr().assistant_custom_invalid(&parsed.ignored.join(", "))
                .into()
        });
        let generation = self.agent.custom_generation.get() + 1;
        self.agent.custom_generation.set(generation);
        let counter = self.agent.custom_generation.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AGENT_SAVE_DELAY).await;
            if counter.get() != generation {
                return;
            }
            let agents = parsed.agents;
            let _ = this.update(cx, |this, cx| {
                this.agent.custom_baseline = agents.clone();
                if settings(cx).agent.custom != agents {
                    update_settings(cx, move |s| s.agent.custom = agents);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn schedule_instructions_save(&mut self, cx: &mut Context<Self>) {
        let generation = self.agent.instructions_generation.get() + 1;
        self.agent.instructions_generation.set(generation);
        let counter = self.agent.instructions_generation.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AGENT_SAVE_DELAY).await;
            if counter.get() != generation {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let text = this.agent.instructions.read(cx).value().trim().to_string();
                this.agent.instructions_baseline = text.clone();
                if settings(cx).agent.instructions.trim() != text {
                    update_settings(cx, move |s| s.agent.instructions = text);
                }
            });
        })
        .detach();
    }

    fn select_agent(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.agent.generation.set(self.agent.generation.get() + 1);
        self.save_agent_fields(window, cx);
        update_settings(cx, move |s| s.agent.active = id);
        self.reload_agent(window, cx);
    }

    fn reload_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = settings(cx).agent.clone();
        let profile = current.active_profile();
        let switched = profile.id != self.agent.profile.id;
        if switched {
            self.agent.generation.set(self.agent.generation.get() + 1);
            self.agent.status = ForgeStatus::Idle;
            self.agent._check = Task::ready(());
        }
        let mut reloaded = false;
        if profile != self.agent.profile {
            reloaded = true;
            let placeholder = profile.default_command.clone();
            self.agent.command.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            });
            self.agent.profile_name = profile.name.clone().into();
            self.agent.profile = profile;
        }
        let id = self.agent.profile.id.as_str();
        let command = stored(&current.commands, id).to_string();
        let model = stored(&current.models, id).to_string();
        for (input, baseline, saved) in [
            (
                &self.agent.command,
                &mut self.agent.command_baseline,
                command,
            ),
            (&self.agent.model, &mut self.agent.model_baseline, model),
        ] {
            let focused = input.read(cx).focus_handle(cx).contains_focused(window, cx);
            if (focused && !switched) || *baseline == saved {
                continue;
            }
            *baseline = saved.clone();
            reloaded = true;
            input.update(cx, |state, cx| state.set_value(saved, window, cx));
        }
        if reloaded {
            self.refresh_models(false, window, cx);
            self.sync_model_select(window, cx);
        }
        let focused = self
            .agent
            .custom
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if !focused && self.agent.custom_baseline != current.custom {
            self.agent.custom_baseline = current.custom.clone();
            self.agent.custom_invalid = None;
            let text = format_custom_agents(&current.custom);
            self.agent.custom_empty = text.is_empty();
            self.agent
                .custom
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.reload_instructions(window, cx);
    }

    fn reload_instructions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = settings(cx).agent.instructions.trim().to_string();
        let focused = self
            .agent
            .instructions
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if focused || self.agent.instructions_baseline == saved {
            return;
        }
        self.agent.instructions_baseline = saved.clone();
        self.agent
            .instructions
            .update(cx, |state, cx| state.set_value(saved, window, cx));
    }

    fn check_agent(&mut self, cx: &mut Context<Self>) {
        let kind = self.agent.profile.kind;
        let typed = self.agent.command.read(cx).value().trim().to_string();
        let command = if typed.is_empty() {
            self.agent.profile.default_command.clone()
        } else {
            typed
        };
        self.agent._check = cx.spawn(async move |this, cx| {
            let result = agent::probe(&command, kind).await;
            let _ = this.update(cx, |this, cx| {
                this.agent.status = match result {
                    Ok(version) => ForgeStatus::Ok(version.into()),
                    Err(err) => ForgeStatus::Failed(err.to_string().into()),
                };
                cx.notify();
            });
        });
        self.agent.status = ForgeStatus::Checking;
        cx.notify();
    }

    fn schedule_hidden_save(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        let rules = match &root {
            None => &mut self.hidden_global,
            Some(root) => match self.repo_rules.iter_mut().find(|r| &r.root == root) {
                Some(entry) => &mut entry.rules,
                None => return,
            },
        };
        let value = rules.input.read(cx).value();
        rules.empty = value.is_empty();
        let typed: Vec<String> = value
            .lines()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.trim().is_empty())
            .collect();
        rules.invalid = invalid_patterns(&typed);
        let baseline = rules.baseline.clone();
        let generation = rules.generation.get() + 1;
        rules.generation.set(generation);
        let counter = rules.generation.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIDDEN_SAVE_DELAY).await;
            if counter.get() != generation {
                return;
            }
            let saved = typed.clone();
            cx.update(|cx| {
                let root = root.clone();
                update_settings(cx, move |s| match root {
                    None => {
                        review_core::hidden::merge_patterns(&mut s.hidden_files, &baseline, &typed)
                    }
                    Some(root) => {
                        let key = repo_key(&root);
                        let list = s.repo_hidden_files.entry(key.clone()).or_default();
                        review_core::hidden::merge_patterns(list, &baseline, &typed);
                        if list.is_empty() {
                            s.repo_hidden_files.remove(&key);
                        }
                    }
                })
            });
            let _ = this.update(cx, |this, _| match &root {
                None => this.hidden_global.baseline = saved,
                Some(root) => {
                    if let Some(entry) = this.repo_rules.iter_mut().find(|r| &r.root == root) {
                        entry.rules.baseline = saved;
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn import_zed_theme(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = path.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(tr().zed_theme_prompt.clone()),
            })
        });
        let registered: HashSet<String> = ThemeRegistry::global(cx)
            .themes()
            .keys()
            .map(|name| name.to_string())
            .collect();
        let window_handle = window.window_handle();
        self._zed_import = cx.spawn(async move |_, cx| {
            let path = match (path, picker) {
                (Some(path), _) => path,
                (None, Some(picker)) => match picker.await {
                    Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                        Some(path) => path,
                        None => return,
                    },
                    Ok(Err(err)) => {
                        log::warn!("zed theme picker: {err:#}");
                        return;
                    }
                    _ => return,
                },
                (None, None) => return,
            };
            let result = cx
                .background_spawn(async move {
                    let dir = ui_resources::themes::themes_dir()
                        .ok_or_else(|| anyhow::anyhow!("data dir unavailable"))?;
                    ui_resources::themes::import_zed_theme(&path, &dir, &registered)
                })
                .await;
            let notification = match result {
                Ok(converted) => {
                    let names: Vec<&str> =
                        converted.themes.iter().map(|t| t.name.as_str()).collect();
                    Notification::success(tr().zed_imported(&names.join(", ")))
                }
                Err(err) => {
                    log::warn!("zed theme import: {err:#}");
                    Notification::error(tr().zed_import_failed(&format!("{err:#}")))
                }
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                window.push_notification(notification, cx);
            });
        });
    }

    fn form(&self, kind: ForgeKind) -> &ForgeForm {
        match kind {
            ForgeKind::GitLab => &self.gitlab,
            ForgeKind::GitHub => &self.github,
        }
    }

    fn form_mut(&mut self, kind: ForgeKind) -> &mut ForgeForm {
        match kind {
            ForgeKind::GitLab => &mut self.gitlab,
            ForgeKind::GitHub => &mut self.github,
        }
    }

    fn save_forge(&mut self, kind: ForgeKind, cx: &mut Context<Self>) {
        let form = self.form(kind);
        let url = form
            .url
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        let token = form.token.read(cx).value().trim().to_string();
        update_settings(cx, |s| match kind {
            ForgeKind::GitLab => {
                s.gitlab_url = url;
                s.gitlab_token = token;
            }
            ForgeKind::GitHub => {
                s.github_url = url;
                s.github_token = token;
            }
        });
        let current = settings(cx);
        if !current.forge_configured(kind) {
            self.form_mut(kind).status = ForgeStatus::Idle;
            cx.notify();
            return;
        }
        let client =
            match forge::Client::new(kind, &current.forge_url(kind), current.forge_token(kind)) {
                Ok(client) => client,
                Err(err) => {
                    self.form_mut(kind).status = ForgeStatus::Failed(err.to_string().into());
                    cx.notify();
                    return;
                }
            };
        let task = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move { client.current_user().await }).await;
            let _ = this.update(cx, |this, cx| {
                this.form_mut(kind).status = match result {
                    Ok(user) => ForgeStatus::Ok(tr().connected_as(&user.username).into()),
                    Err(err) => ForgeStatus::Failed(err.to_string().into()),
                };
                cx.notify();
            });
        });
        let form = self.form_mut(kind);
        form.status = ForgeStatus::Checking;
        form._check = task;
        cx.notify();
    }
}

fn forge_group(view: &Entity<SettingsView>, kind: ForgeKind, s: &Strings) -> SettingGroup {
    let (description, token_description) = match kind {
        ForgeKind::GitLab => (s.gitlab_desc.clone(), s.gitlab_token_desc.clone()),
        ForgeKind::GitHub => (s.github_desc.clone(), s.github_token_desc.clone()),
    };
    let url_view = view.clone();
    let token_view = view.clone();
    let status_view = view.clone();
    SettingGroup::new()
        .title(kind.name())
        .description(description)
        .item(
            SettingItem::new(
                s.server_url.clone(),
                SettingField::render(move |_, _, cx| {
                    div()
                        .w_full()
                        .max_w(px(FIELD_MAX_WIDTH))
                        .child(Input::new(&url_view.read(cx).form(kind).url).small())
                }),
            )
            .layout(Axis::Vertical),
        )
        .item(
            SettingItem::new(
                s.access_token.clone(),
                SettingField::render(move |_, _, cx| {
                    let form = token_view.read(cx).form(kind);
                    let typed_url = form.url.read(cx).value();
                    h_flex()
                        .w_full()
                        .max_w(px(FIELD_MAX_WIDTH))
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&form.token).small()),
                        )
                        .children(kind.new_token_url(&typed_url).map(|url| {
                            Button::new(match kind {
                                ForgeKind::GitLab => "gitlab-new-token",
                                ForgeKind::GitHub => "github-new-token",
                            })
                            .small()
                            .label(tr().create_token.clone())
                            .tooltip(tr().create_token_tooltip.clone())
                            .on_click(move |_, _, cx| cx.open_url(&url))
                        }))
                }),
            )
            .layout(Axis::Vertical)
            .description(token_description),
        )
        .item(SettingItem::render(move |_, _, cx| {
            let status = status_view.read(cx).form(kind).status.clone();
            let save_view = status_view.clone();
            h_flex()
                .w_full()
                .gap_2()
                .justify_start()
                .items_center()
                .child(
                    Button::new(match kind {
                        ForgeKind::GitLab => "gitlab-save",
                        ForgeKind::GitHub => "github-save",
                    })
                    .primary()
                    .small()
                    .icon(IconName::Check)
                    .label(tr().save_and_check.clone())
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |this, cx| this.save_forge(kind, cx))
                    }),
                )
                .child(status_line(status, cx))
        }))
}

fn status_line(status: ForgeStatus, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (text, color) = match status {
        ForgeStatus::Idle => return div().into_any_element(),
        ForgeStatus::Checking => (tr().checking.clone(), theme.muted_foreground),
        ForgeStatus::Ok(text) => (text, theme.success),
        ForgeStatus::Failed(text) => (text, theme.danger),
    };
    div()
        .text_sm()
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn agent_dropdown(view: Entity<SettingsView>, cx: &App) -> AnyElement {
    let label = view.read(cx).agent.profile_name.clone();
    Button::new("settings-agent")
        .small()
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let agent = &settings(cx).agent;
            let current = agent.active_profile().id;
            agent.profiles().into_iter().fold(menu, |menu, profile| {
                let checked = profile.id == current;
                let view = view.clone();
                let id = profile.id;
                menu.item(PopupMenuItem::new(profile.name).checked(checked).on_click(
                    move |_, window, cx| {
                        let id = id.clone();
                        view.update(cx, |this, cx| this.select_agent(id, window, cx))
                    },
                ))
            })
        })
        .into_any_element()
}

fn agent_custom_field(view: &Entity<SettingsView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let form = &view.read(cx).agent;
    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .relative()
                .w_full()
                .max_w(px(FIELD_MAX_WIDTH))
                .child(Textarea::new(&form.custom).font_family(theme.mono_font_family.clone()))
                .when(form.custom_empty, |field| {
                    field.child(
                        div()
                            .absolute()
                            .top(px(9.))
                            .left(px(11.))
                            .text_sm()
                            .line_height(rems(1.25))
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground.opacity(0.45))
                            .child(AGENT_CUSTOM_EXAMPLE),
                    )
                }),
        )
        .children(
            form.custom_invalid
                .clone()
                .map(|invalid| div().text_xs().text_color(theme.danger).child(invalid)),
        )
        .into_any_element()
}

fn agent_model_field(view: &Entity<SettingsView>, cx: &App) -> AnyElement {
    let form = &view.read(cx).agent;
    let theme = cx.theme();
    let s = tr();
    let refresh_view = view.clone();
    let loading = form.catalog == Catalog::Loading;
    v_flex()
        .w_full()
        .max_w(px(FIELD_MAX_WIDTH))
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .items_center()
                .child(div().flex_1().min_w_0().map(|el| {
                    match form.catalog {
                        Catalog::Ready => el.child(
                            Select::new(&form.model_select)
                                .small()
                                .w_full()
                                .search_placeholder(s.assistant_model_search.clone()),
                        ),
                        Catalog::Loading => el.child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(s.assistant_model_loading.clone()),
                        ),
                        Catalog::Failed => el,
                    }
                }))
                .child(
                    Button::new("agent-models-refresh")
                        .ghost()
                        .small()
                        .icon(IconName::RefreshCw)
                        .tooltip(s.assistant_model_refresh.clone())
                        .loading(loading)
                        .disabled(loading)
                        .on_click(move |_, window, cx| {
                            refresh_view
                                .update(cx, |this, cx| this.refresh_models(true, window, cx))
                        }),
                ),
        )
        .child(Input::new(&form.model).small())
        .into_any_element()
}

fn agent_page(view: &Entity<SettingsView>, profile: &Profile, s: &Strings) -> SettingPage {
    let dropdown_view = view.clone();
    let command_view = view.clone();
    let model_view = view.clone();
    let check_view = view.clone();
    let custom_view = view.clone();
    let instructions_view = view.clone();
    let general = SettingGroup::new()
        .title(s.general.clone())
        .description(s.assistant_desc.clone())
        .item(
            SettingItem::new(
                s.assistant_show.clone(),
                SettingField::switch(
                    |cx| settings(cx).agent.enabled,
                    |value, cx| update_settings(cx, |s| s.agent.enabled = value),
                ),
            )
            .description(s.assistant_show_desc.clone()),
        )
        .item(
            SettingItem::new(
                s.assistant_auto_overview.clone(),
                SettingField::switch(
                    |cx| settings(cx).agent.auto_overview,
                    |value, cx| update_settings(cx, |s| s.agent.auto_overview = value),
                ),
            )
            .description(s.assistant_auto_overview_desc.clone()),
        );
    let mut agents = SettingGroup::new()
        .title(s.assistant_agents.clone())
        .item(
            SettingItem::new(
                s.assistant_agent.clone(),
                SettingField::render(move |_, _, cx| agent_dropdown(dropdown_view.clone(), cx)),
            )
            .description(s.assistant_agent_desc.clone()),
        )
        .item(
            SettingItem::new(
                s.assistant_command.clone(),
                SettingField::render(move |_, _, cx| {
                    div()
                        .w_full()
                        .max_w(px(FIELD_MAX_WIDTH))
                        .child(Input::new(&command_view.read(cx).agent.command).small())
                }),
            )
            .layout(Axis::Vertical)
            .description(s.assistant_command_desc.clone()),
        )
        .item(
            SettingItem::new(
                s.assistant_model.clone(),
                SettingField::render(move |_, _, cx| agent_model_field(&model_view, cx)),
            )
            .layout(Axis::Vertical)
            .description(s.assistant_model_desc.clone()),
        )
        .item(SettingItem::render(move |_, _, cx| {
            let status = check_view.read(cx).agent.status.clone();
            let checking = matches!(status, ForgeStatus::Checking);
            let click_view = check_view.clone();
            h_flex()
                .w_full()
                .gap_2()
                .justify_start()
                .items_center()
                .child(
                    Button::new("agent-check")
                        .small()
                        .icon(IconName::Check)
                        .label(tr().assistant_check.clone())
                        .loading(checking)
                        .disabled(checking)
                        .on_click(move |_, _, cx| {
                            click_view.update(cx, |this, cx| this.check_agent(cx))
                        }),
                )
                .child(status_line(status, cx))
        }));
    if !profile.has_sandbox() {
        let message = s.assistant_no_sandbox.clone();
        agents = agents.item(SettingItem::render(move |_, _, cx| {
            div()
                .text_sm()
                .text_color(cx.theme().warning)
                .child(message.clone())
        }));
    }
    let agents = agents.item(
        SettingItem::new(
            s.assistant_custom.clone(),
            SettingField::render(move |_, _, cx| agent_custom_field(&custom_view, cx)),
        )
        .layout(Axis::Vertical)
        .description(s.assistant_custom_desc.clone()),
    );
    let instructions = SettingGroup::new()
        .title(s.assistant_instructions_group.clone())
        .item(
            SettingItem::new(
                s.assistant_instructions.clone(),
                SettingField::render(move |_, _, cx| {
                    div()
                        .w_full()
                        .max_w(px(FIELD_MAX_WIDTH))
                        .child(Textarea::new(
                            &instructions_view.read(cx).agent.instructions,
                        ))
                }),
            )
            .layout(Axis::Vertical)
            .description(s.assistant_instructions_desc.clone()),
        );
    SettingPage::new(s.assistant.clone())
        .group(general)
        .group(agents)
        .group(instructions)
}

fn theme_label(choice: &ThemeChoice) -> SharedString {
    match choice {
        ThemeChoice::System => tr().system.clone(),
        ThemeChoice::Named(name) => SharedString::from(name.clone()),
    }
}

fn editor_presets_button() -> AnyElement {
    Button::new("editor-presets")
        .small()
        .label(tr().editor_presets.clone())
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, |menu, _, cx| {
            let current = settings(cx).editor_command.clone();
            editor_presets(HostOs::current())
                .into_iter()
                .fold(menu, |menu, (name, template)| {
                    let checked = template == current;
                    menu.item(PopupMenuItem::new(name).checked(checked).on_click(
                        move |_, _, cx| {
                            let template = template.clone();
                            update_settings(cx, move |s| s.editor_command = template);
                        },
                    ))
                })
        })
        .into_any_element()
}

fn version_field(cx: &App) -> AnyElement {
    let state = updater::handle(cx).map(|updater| updater.read(cx).state());
    let busy = matches!(
        state,
        Some(UpdateState::Checking | UpdateState::Downloading(_))
    );
    v_flex()
        .items_end()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(env!("CARGO_PKG_VERSION")),
                )
                .when(state.is_some(), |el| {
                    el.child(
                        Button::new("check-for-updates")
                            .small()
                            .label(tr().check_for_updates.clone())
                            .loading(busy)
                            .disabled(busy)
                            .on_click(|_, _, cx| updater::check_now(cx)),
                    )
                }),
        )
        .children(state.and_then(|state| update_status(state, cx)))
        .into_any_element()
}

fn update_status(state: UpdateState, cx: &App) -> Option<AnyElement> {
    let s = tr();
    let theme = cx.theme();
    let (text, color): (SharedString, _) = match state {
        UpdateState::Idle => return None,
        UpdateState::Checking => (s.checking_for_updates.clone(), theme.muted_foreground),
        UpdateState::Downloading(version) => (
            s.downloading_update(&version).into(),
            theme.muted_foreground,
        ),
        UpdateState::UpToDate => (s.up_to_date.clone(), theme.success),
        UpdateState::Failed(error) => (s.update_check_failed(&error).into(), theme.danger),
        UpdateState::Ready(version) => {
            return Some(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_xs().child(s.update_ready(&version)))
                    .child(
                        Button::new("restart-to-update")
                            .small()
                            .primary()
                            .label(s.restart_to_update.clone())
                            .on_click(|_, _, cx| updater::apply_and_restart(cx)),
                    )
                    .into_any_element(),
            );
        }
    };
    Some(
        div()
            .max_w(px(360.))
            .text_xs()
            .text_right()
            .text_color(color)
            .child(text)
            .into_any_element(),
    )
}

fn theme_dropdown(cx: &App) -> AnyElement {
    Button::new("settings-theme")
        .small()
        .label(theme_label(&settings(cx).theme))
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, |menu, _, cx| {
            let current = settings(cx).theme.clone();
            let mut themes = ThemeRegistry::global(cx).sorted_themes();
            themes.sort_by(|a, b| {
                b.mode
                    .is_dark()
                    .cmp(&a.mode.is_dark())
                    .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            let options = std::iter::once(ThemeChoice::System).chain(
                themes
                    .into_iter()
                    .map(|t| ThemeChoice::Named(t.name.to_string())),
            );
            options.enumerate().fold(
                menu.max_h(px(420.)).scrollable(true),
                |menu, (ix, choice)| {
                    let checked = choice == current;
                    let label = theme_label(&choice);
                    let preview = choice.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, _| {
                            let preview = preview.clone();
                            div()
                                .id(("theme-option", ix))
                                .child(label.clone())
                                .on_hover(move |hovered, _, cx| {
                                    if *hovered {
                                        apply_theme(&preview, cx);
                                    }
                                })
                        })
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            let choice = choice.clone();
                            update_settings(cx, |s| s.theme = choice.clone());
                            apply_theme(&choice, cx);
                        }),
                    )
                },
            )
        })
        .on_open_change(|open, _, cx| {
            if !*open {
                apply_saved_theme(cx);
            }
        })
        .into_any_element()
}

fn syntax_label(choice: Option<&String>) -> SharedString {
    match choice {
        None => tr().same_as_theme.clone(),
        Some(name) => SharedString::from(name.clone()),
    }
}

fn syntax_dropdown(cx: &App) -> AnyElement {
    let dark = cx.theme().is_dark();
    Button::new("settings-syntax-colors")
        .small()
        .label(syntax_label(settings(cx).syntax_colors.get(dark)))
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let current = settings(cx).syntax_colors.get(dark).cloned();
            let options =
                std::iter::once(None).chain(syntax_palette_names(dark, cx).into_iter().map(Some));
            options.enumerate().fold(
                menu.max_h(px(420.)).scrollable(true),
                |menu, (ix, choice)| {
                    let checked = choice == current;
                    let label = syntax_label(choice.as_ref());
                    let preview = choice.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, _| {
                            let preview = preview.clone();
                            div()
                                .id(("syntax-option", ix))
                                .child(label.clone())
                                .on_hover(move |hovered, _, cx| {
                                    if *hovered {
                                        preview_syntax_colors(preview.clone(), cx);
                                    }
                                })
                        })
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            let choice = choice.clone();
                            update_settings(cx, |s| s.syntax_colors.set(dark, choice));
                            end_syntax_preview(cx);
                        }),
                    )
                },
            )
        })
        .on_open_change(|open, _, cx| {
            if !*open {
                end_syntax_preview(cx);
            }
        })
        .into_any_element()
}

fn import_zed_button(view: Entity<SettingsView>, cx: &App) -> AnyElement {
    let files = view.read(cx).zed_files.clone();
    let button = Button::new("settings-import-zed")
        .small()
        .label(tr().import_zed_theme.clone());
    if files.is_empty() {
        return button
            .on_click(move |_, window, cx| {
                view.update(cx, |this, cx| this.import_zed_theme(None, window, cx))
            })
            .into_any_element();
    }
    button
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let menu = files.iter().fold(
                menu.max_h(px(420.))
                    .scrollable(true)
                    .label(tr().installed_in_zed.clone()),
                |menu, file| {
                    let path = file.path.clone();
                    let view = view.clone();
                    menu.item(PopupMenuItem::new(file.label.clone()).on_click(
                        move |_, window, cx| {
                            let path = path.clone();
                            view.update(cx, |this, cx| {
                                this.import_zed_theme(Some(path), window, cx)
                            })
                        },
                    ))
                },
            );
            let view = view.clone();
            menu.separator()
                .item(PopupMenuItem::new(tr().choose_file.clone()).on_click(
                    move |_, window, cx| {
                        view.update(cx, |this, cx| this.import_zed_theme(None, window, cx))
                    },
                ))
        })
        .into_any_element()
}

fn lang_label(choice: &LangChoice) -> SharedString {
    match choice {
        LangChoice::System => tr().system.clone(),
        LangChoice::Named(code) => Lang::from_code(code)
            .map(|l| SharedString::from(l.display_name()))
            .unwrap_or_else(|| SharedString::from(code.clone())),
    }
}

fn language_dropdown(cx: &App) -> AnyElement {
    Button::new("settings-language")
        .small()
        .label(lang_label(&settings(cx).language))
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, |menu, _, cx| {
            let current = settings(cx).language.clone();
            std::iter::once(LangChoice::System)
                .chain(
                    Lang::all()
                        .iter()
                        .map(|l| LangChoice::Named(l.code().to_string())),
                )
                .fold(menu, |menu, choice| {
                    let checked = choice == current;
                    menu.item(
                        PopupMenuItem::new(lang_label(&choice))
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let choice = choice.clone();
                                update_settings(cx, |s| s.language = choice);
                                sync_active_lang(cx);
                                crate::app_menu::set_menus(cx);
                                cx.refresh_windows();
                            }),
                    )
                })
        })
        .into_any_element()
}

fn code_size_control(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let zoom = settings(cx).code_zoom;
    let size = f32::from(theme.mono_font_size).round() as i32;
    h_flex()
        .gap_1()
        .items_center()
        .child(
            Button::new("code-size-down")
                .outline()
                .small()
                .icon(IconName::Minus)
                .tooltip_with_action(tr().cmd_zoom_out.clone(), &ZoomOut, None)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ZoomOut), cx)),
        )
        .child(
            div()
                .w(px(44.))
                .text_sm()
                .text_center()
                .child(format!("{size} px")),
        )
        .child(
            Button::new("code-size-up")
                .outline()
                .small()
                .icon(IconName::Plus)
                .tooltip_with_action(tr().cmd_zoom_in.clone(), &ZoomIn, None)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ZoomIn), cx)),
        )
        .when(zoom != 0, |el| {
            el.child(
                Button::new("code-size-reset")
                    .ghost()
                    .small()
                    .icon(Icon::new(Icons::Undo2))
                    .tooltip_with_action(tr().cmd_zoom_reset.clone(), &ZoomReset, None)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ZoomReset), cx)),
            )
        })
        .into_any_element()
}

fn choice_buttons<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: Vec<(SharedString, T)>,
    current: T,
    set: fn(&mut crate::settings_store::Settings, T),
) -> AnyElement {
    let values: Vec<T> = options.iter().map(|(_, v)| *v).collect();
    let mut group = gpui_component::button::ButtonGroup::new(id)
        .small()
        .outline();
    for (ix, (label, value)) in options.into_iter().enumerate() {
        group = group.child(
            Button::new((id, ix))
                .label(label)
                .selected(value == current),
        );
    }
    group
        .on_click(move |clicks: &Vec<usize>, _, cx| {
            if let Some(value) = clicks.first().and_then(|ix| values.get(*ix)).copied() {
                update_settings(cx, |s| set(s, value));
            }
        })
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.keep_focus_inside(window, cx);
        let s = tr();
        let view = cx.entity();
        let syntax_desc = if cx.theme().is_dark() {
            s.syntax_colors_dark_desc.clone()
        } else {
            s.syntax_colors_light_desc.clone()
        };
        let import_view = view.clone();
        let editor_view = view.clone();
        let mut general = SettingGroup::new()
            .title(s.general.clone())
            .item(SettingItem::new(
                s.language.clone(),
                SettingField::render(|_, _, cx| language_dropdown(cx)),
            ));
        if updater::is_supported() {
            general = general.item(
                SettingItem::new(
                    s.automatic_updates.clone(),
                    SettingField::switch(
                        |cx| settings(cx).auto_update,
                        |value, cx| {
                            update_settings(cx, |s| s.auto_update = value);
                            updater::set_enabled(cx, value);
                        },
                    ),
                )
                .description(s.automatic_updates_desc.clone()),
            );
        }
        let general = general.item(SettingItem::new(
            s.version.clone(),
            SettingField::render(|_, _, cx| version_field(cx)),
        ));
        let appearance = SettingGroup::new()
            .title(s.appearance.clone())
            .item(
                SettingItem::new(
                    s.theme.clone(),
                    SettingField::render(|_, _, cx| theme_dropdown(cx)),
                )
                .description(s.theme_desc.clone()),
            )
            .item(
                SettingItem::new(
                    s.syntax_colors.clone(),
                    SettingField::render(|_, _, cx| syntax_dropdown(cx)),
                )
                .description(syntax_desc),
            )
            .item(SettingItem::new(
                s.zed_themes.clone(),
                SettingField::render(move |_, _, cx| import_zed_button(import_view.clone(), cx)),
            ))
            .item(SettingItem::new(
                s.interface_size.clone(),
                SettingField::render(|_, _, cx| {
                    choice_buttons(
                        "ui-scale",
                        UI_SCALES
                            .iter()
                            .map(|scale| (SharedString::from(format!("{scale}%")), *scale))
                            .collect(),
                        settings(cx).ui_scale,
                        |s, value| s.ui_scale = value,
                    )
                }),
            ))
            .item(
                SettingItem::new(
                    s.code_text_size.clone(),
                    SettingField::render(|_, _, cx| code_size_control(cx)),
                )
                .description(SharedString::from(
                    s.code_text_size_desc(
                        &["window.zoom_in", "window.zoom_out", "window.zoom_reset"]
                            .iter()
                            .filter_map(|id| crate::keymap::shown_key(id, cx))
                            .collect::<Vec<_>>()
                            .join(" / "),
                    ),
                )),
            );
        let review =
            SettingGroup::new()
                .title(s.review.clone())
                .item(SettingItem::new(
                    s.tree_side.clone(),
                    SettingField::render(|_, _, cx| {
                        let s = tr();
                        choice_buttons(
                            "tree-side",
                            vec![
                                (s.left.clone(), TreeSide::Left),
                                (s.right.clone(), TreeSide::Right),
                            ],
                            settings(cx).tree_side,
                            |st, v| st.tree_side = v,
                        )
                    }),
                ))
                .item(
                    SettingItem::new(
                        s.show_whole_file.clone(),
                        SettingField::switch(
                            |cx| settings(cx).show_whole_file,
                            |value, cx| update_settings(cx, |s| s.show_whole_file = value),
                        ),
                    )
                    .description(s.show_whole_file_desc.clone()),
                )
                .item(
                    SettingItem::new(
                        s.wrap_lines.clone(),
                        SettingField::render(|_, _, cx| {
                            let s = tr();
                            choice_buttons(
                                "wrap-lines",
                                vec![
                                    (s.wrap_split_only.clone(), WrapLines::SplitOnly),
                                    (s.wrap_always.clone(), WrapLines::Always),
                                    (s.wrap_never.clone(), WrapLines::Never),
                                ],
                                settings(cx).wrap_lines,
                                |st, v| st.wrap_lines = v,
                            )
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description(s.wrap_lines_desc.clone()),
                )
                .item(
                    SettingItem::new(
                        s.ignore_whitespace.clone(),
                        SettingField::switch(
                            |cx| settings(cx).ignore_whitespace,
                            |value, cx| update_settings(cx, |s| s.ignore_whitespace = value),
                        ),
                    )
                    .description(s.ignore_whitespace_desc.clone()),
                )
                .item(SettingItem::new(
                    s.collapse_viewed_folders.clone(),
                    SettingField::switch(
                        |cx| settings(cx).collapse_viewed_folders,
                        |value, cx| update_settings(cx, |s| s.collapse_viewed_folders = value),
                    ),
                ))
                .item(
                    SettingItem::new(
                        s.editor_command.clone(),
                        SettingField::render(move |_, _, cx| {
                            h_flex()
                                .w_full()
                                .max_w(px(FIELD_MAX_WIDTH))
                                .gap_1()
                                .child(div().flex_1().min_w_0().child(
                                    Input::new(&editor_view.read(cx).editor_command).small(),
                                ))
                                .child(editor_presets_button())
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description(s.editor_command_desc.clone()),
                );
        let assistant = agent_page(&view, &self.agent.profile, s);
        let gitlab = forge_group(&view, ForgeKind::GitLab, s);
        let github = forge_group(&view, ForgeKind::GitHub, s);
        let global_view = view.clone();
        let mut hidden = SettingGroup::new()
            .title(s.hidden_files_setting.clone())
            .description(s.hidden_files_setting_desc.clone())
            .item(SettingItem::render(move |_, _, cx| {
                let this = global_view.read(cx);
                hidden_rules_item(
                    tr().hidden_files_global.clone(),
                    tr().hidden_files_global_desc.clone(),
                    &this.hidden_global,
                    cx,
                )
            }));
        for (ix, entry) in self.repo_rules.iter().enumerate() {
            let title: SharedString = s.hidden_files_repo(&entry.name).into();
            let repo_view = view.clone();
            hidden = hidden.item(SettingItem::render(move |_, _, cx| {
                let this = repo_view.read(cx);
                match this.repo_rules.get(ix) {
                    Some(entry) => {
                        hidden_rules_item(title.clone(), entry.path.clone(), &entry.rules, cx)
                    }
                    None => div().into_any_element(),
                }
            }));
        }
        v_flex()
            .id("settings-screen")
            .key_context("SettingsScreen")
            .track_focus(&self.focus)
            .on_action(|_: &CloseSettings, window, _| window.remove_window())
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &ShowSettings, window, _| window.activate_window())
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new()
                    .when(window.is_fullscreen(), |bar| bar.pl(px(0.)))
                    .child(div().text_sm().font_semibold().child(s.settings.clone())),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new(ElementId::NamedInteger(
                        "nitpick-settings".into(),
                        self.generation,
                    ))
                    .when_some(self.revealed, |settings, ix| {
                        settings.default_selected_index(ix)
                    })
                    .pages([
                        SettingPage::new(s.settings.clone())
                            .group(general)
                            .group(appearance)
                            .group(review)
                            .group(gitlab)
                            .group(github),
                        SettingPage::new(s.hidden_files_setting.clone()).group(hidden),
                        SettingPage::new(s.shortcuts.clone())
                            .group(shortcut_group(s.shortcuts_review.clone(), Group::Review, s))
                            .group(shortcut_group(s.shortcuts_tree.clone(), Group::Tree, s))
                            .group(shortcut_group(s.shortcuts_window.clone(), Group::Window, s)),
                        assistant,
                    ]),
                ),
            )
    }
}

#[cfg(test)]
#[path = "settings_view_tests.rs"]
mod tests;
