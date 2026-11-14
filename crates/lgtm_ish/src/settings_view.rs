use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Anchor, AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, actions, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _, ThemeRegistry,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    setting::{SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use ui_resources::Icons;
use ui_resources::i18n::{Lang, Strings};
use ui_resources::themes::ZedThemeFile;

use crate::keymap::{self, COMMANDS, Command, Group};
use crate::localization::{sync_active_lang, tr};
use crate::review::{HostOs, editor_presets};
use crate::settings_store::{
    DiffLayout, LangChoice, SettingsStore, ThemeChoice, TreeSide, UI_SCALES, WrapLines,
    apply_saved_theme, apply_theme, end_syntax_preview, preview_syntax_colors, repo_key, settings,
    syntax_palette_names, update_settings,
};
use crate::shortcut_dialog::{self, keystroke_element, where_label};
use crate::zoom::{ZoomIn, ZoomOut, ZoomReset};

actions!(settings_view, [CloseSettings]);

const HIDDEN_SAVE_DELAY: Duration = Duration::from_millis(400);
const EDITOR_SAVE_DELAY: Duration = Duration::from_millis(400);

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
        .child(Textarea::new(&rules.input).font_family(theme.mono_font_family.clone()))
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
enum GitlabStatus {
    Idle,
    Checking,
    Ok(SharedString),
    Failed(SharedString),
}

struct HiddenRules {
    input: Entity<TextareaState>,
    invalid: Option<SharedString>,
    baseline: Vec<String>,
    generation: Rc<Cell<u64>>,
}

impl HiddenRules {
    fn new(input: Entity<TextareaState>) -> Self {
        Self {
            input,
            invalid: None,
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
        self.input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }
}

pub enum SettingsEvent {
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    HiddenFiles,
}

const HIDDEN_FILES_PAGE: usize = 1;

impl EventEmitter<SettingsEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub struct SettingsView {
    focus: FocusHandle,
    generation: u64,
    revealed: Option<SelectIndex>,
    gitlab_url: Entity<InputState>,
    gitlab_token: Entity<InputState>,
    editor_command: Entity<InputState>,
    editor_generation: Rc<Cell<u64>>,
    editor_baseline: String,
    _editor_subscription: Subscription,
    hidden_global: HiddenRules,
    hidden_repo: HiddenRules,
    repo: Option<(PathBuf, SharedString)>,
    _hidden_subscriptions: [Subscription; 2],
    _settings_sync: Option<Subscription>,
    status: GitlabStatus,
    zed_files: Rc<Vec<ZedThemeFile>>,
    _check: Task<()>,
    _zed_discovery: Task<()>,
    _zed_import: Task<()>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = settings(cx).clone();
        let gitlab_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://gitlab.example.com")
                .default_value(current.gitlab_url.clone())
        });
        let gitlab_token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("glpat-…")
                .masked(true)
                .default_value(current.gitlab_token.clone())
        });
        let editor_command = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("code -g {file}:{line}")
                .default_value(current.editor_command.clone())
        });
        let editor_subscription =
            cx.subscribe(&editor_command, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_editor_save(cx);
                }
            });
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
            cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 12)
                    .placeholder("*_mock.go\nvendor/")
            })
        };
        let global_input = hidden_input(window, cx);
        let repo_input = hidden_input(window, cx);
        let subscriptions = [
            cx.subscribe(&global_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_hidden_save(false, cx);
                }
            }),
            cx.subscribe(&repo_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_hidden_save(true, cx);
                }
            }),
        ];
        let mut this = Self {
            focus: cx.focus_handle(),
            generation: 0,
            revealed: None,
            gitlab_url,
            gitlab_token,
            editor_command,
            editor_generation: Rc::default(),
            editor_baseline: current.editor_command.clone(),
            _editor_subscription: editor_subscription,
            hidden_global: HiddenRules::new(global_input),
            hidden_repo: HiddenRules::new(repo_input),
            repo: None,
            _hidden_subscriptions: subscriptions,
            _settings_sync: None,
            status: GitlabStatus::Idle,
            zed_files: Rc::default(),
            _check: Task::ready(()),
            _zed_discovery: zed_discovery,
            _zed_import: Task::ready(()),
        };
        this.reload_hidden(true, window, cx);
        this._settings_sync = Some(cx.observe_global_in::<SettingsStore>(
            window,
            |this, window, cx| {
                this.reload_hidden(false, window, cx);
                this.reload_editor_command(window, cx);
                cx.notify();
            },
        ));
        this
    }

    pub fn reveal(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        let page_ix = match section {
            SettingsSection::HiddenFiles => HIDDEN_FILES_PAGE,
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
        let repo = self
            .repo
            .as_ref()
            .map(|(root, _)| current.repo_hidden(root).to_vec())
            .unwrap_or_default();
        self.hidden_repo.load(&repo, force, window, cx);
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

    fn schedule_hidden_save(&mut self, repository: bool, cx: &mut Context<Self>) {
        let root = self.repo.as_ref().map(|(root, _)| root.clone());
        let rules = if repository {
            &mut self.hidden_repo
        } else {
            &mut self.hidden_global
        };
        let typed: Vec<String> = rules
            .input
            .read(cx)
            .value()
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
                update_settings(cx, move |s| match (repository, root) {
                    (false, _) => {
                        review_core::hidden::merge_patterns(&mut s.hidden_files, &baseline, &typed)
                    }
                    (true, Some(root)) => {
                        let key = repo_key(&root);
                        let list = s.repo_hidden_files.entry(key.clone()).or_default();
                        review_core::hidden::merge_patterns(list, &baseline, &typed);
                        if list.is_empty() {
                            s.repo_hidden_files.remove(&key);
                        }
                    }
                    (true, None) => {}
                })
            });
            let _ = this.update(cx, |this, _| {
                if repository {
                    this.hidden_repo.baseline = saved;
                } else {
                    this.hidden_global.baseline = saved;
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

    fn save_gitlab(&mut self, cx: &mut Context<Self>) {
        let url = self
            .gitlab_url
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        let token = self.gitlab_token.read(cx).value().trim().to_string();
        update_settings(cx, |s| {
            s.gitlab_url = url;
            s.gitlab_token = token;
        });
        let Some(client) = settings(cx).gitlab_client() else {
            self.status = GitlabStatus::Idle;
            cx.notify();
            return;
        };
        self.status = GitlabStatus::Checking;
        cx.notify();
        self._check = cx.spawn(async move |this, cx| {
            let result = net_runtime::spawn(async move { client.current_user().await }).await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(user) => GitlabStatus::Ok(tr().gitlab_connected(&user.username).into()),
                    Err(err) => GitlabStatus::Failed(err.to_string().into()),
                };
                cx.notify();
            });
        });
    }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = tr();
        let view = cx.entity();
        let syntax_desc = if cx.theme().is_dark() {
            s.syntax_colors_dark_desc.clone()
        } else {
            s.syntax_colors_light_desc.clone()
        };
        let import_view = view.clone();
        let editor_view = view.clone();
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
            .item(
                SettingItem::new(
                    s.zed_themes.clone(),
                    SettingField::render(move |_, _, cx| {
                        import_zed_button(import_view.clone(), cx)
                    }),
                )
                .description(s.import_zed_theme_desc.clone()),
            )
            .item(SettingItem::new(
                s.language.clone(),
                SettingField::render(|_, _, cx| language_dropdown(cx)),
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
                .description(s.code_text_size_desc.clone()),
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
                .item(SettingItem::new(
                    s.diff_layout.clone(),
                    SettingField::render(|_, _, cx| {
                        let s = tr();
                        choice_buttons(
                            "diff-layout-setting",
                            vec![
                                (s.split_view.clone(), DiffLayout::Split),
                                (s.unified_view.clone(), DiffLayout::Unified),
                            ],
                            settings(cx).diff_layout,
                            |st, v| st.diff_layout = v,
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
                .item(SettingItem::new(
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
                ))
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
                .item(
                    SettingItem::new(
                        s.advance_on_viewed.clone(),
                        SettingField::switch(
                            |cx| settings(cx).advance_on_viewed,
                            |value, cx| update_settings(cx, |s| s.advance_on_viewed = value),
                        ),
                    )
                    .description(s.advance_on_viewed_desc.clone()),
                )
                .item(
                    SettingItem::new(
                        s.auto_mark_viewed.clone(),
                        SettingField::switch(
                            |cx| settings(cx).auto_mark_viewed,
                            |value, cx| update_settings(cx, |s| s.auto_mark_viewed = value),
                        ),
                    )
                    .description(s.auto_mark_viewed_desc.clone()),
                )
                .item(
                    SettingItem::new(
                        s.editor_command.clone(),
                        SettingField::render(move |_, _, cx| {
                            h_flex()
                                .gap_1()
                                .child(div().w(px(320.)).child(
                                    Input::new(&editor_view.read(cx).editor_command).small(),
                                ))
                                .child(editor_presets_button())
                        }),
                    )
                    .description(s.editor_command_desc.clone()),
                );
        let url_view = view.clone();
        let token_view = view.clone();
        let status_view = view.clone();
        let gitlab = SettingGroup::new()
            .title("GitLab")
            .description(s.gitlab_desc.clone())
            .item(SettingItem::new(
                s.gitlab_url.clone(),
                SettingField::render(move |_, _, cx| {
                    div()
                        .w(px(320.))
                        .child(Input::new(&url_view.read(cx).gitlab_url).small())
                }),
            ))
            .item(
                SettingItem::new(
                    s.gitlab_token.clone(),
                    SettingField::render(move |_, _, cx| {
                        div()
                            .w(px(320.))
                            .child(Input::new(&token_view.read(cx).gitlab_token).small())
                    }),
                )
                .description(s.gitlab_token_desc.clone()),
            )
            .item(SettingItem::render(move |_, _, cx| {
                let status = status_view.read(cx).status.clone();
                let theme = cx.theme();
                let save_view = status_view.clone();
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .items_center()
                    .child(match status {
                        GitlabStatus::Idle => div().into_any_element(),
                        GitlabStatus::Checking => div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(tr().checking.clone())
                            .into_any_element(),
                        GitlabStatus::Ok(text) => div()
                            .text_sm()
                            .text_color(theme.success)
                            .child(text)
                            .into_any_element(),
                        GitlabStatus::Failed(text) => div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(text)
                            .into_any_element(),
                    })
                    .child(
                        Button::new("gitlab-save")
                            .primary()
                            .small()
                            .icon(IconName::Check)
                            .label(tr().save_and_check.clone())
                            .on_click(move |_, _, cx| {
                                save_view.update(cx, |this, cx| this.save_gitlab(cx))
                            }),
                    )
            }));
        let global_view = view.clone();
        let repo_view = view.clone();
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
        if let Some((_, name)) = &self.repo {
            let title: SharedString = s.hidden_files_repo(name).into();
            hidden = hidden.item(SettingItem::render(move |_, _, cx| {
                let this = repo_view.read(cx);
                hidden_rules_item(
                    title.clone(),
                    tr().hidden_files_repo_desc.clone(),
                    &this.hidden_repo,
                    cx,
                )
            }));
        }
        v_flex()
            .id("settings-screen")
            .key_context("SettingsScreen")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseSettings, _, cx| cx.emit(SettingsEvent::Close)))
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .h(px(40.))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("settings-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .label(s.back.clone())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    )
                    .child(div().font_semibold().child(s.settings.clone())),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new(ElementId::NamedInteger(
                        "lgtm-ish-settings".into(),
                        self.generation,
                    ))
                    .when_some(self.revealed, |settings, ix| {
                        settings.default_selected_index(ix)
                    })
                    .pages([
                        SettingPage::new(s.settings.clone())
                            .group(appearance)
                            .group(review)
                            .group(gitlab),
                        SettingPage::new(s.hidden_files_setting.clone()).group(hidden),
                        SettingPage::new(s.shortcuts.clone())
                            .group(shortcut_group(s.shortcuts_review.clone(), Group::Review, s))
                            .group(shortcut_group(s.shortcuts_tree.clone(), Group::Tree, s))
                            .group(shortcut_group(s.shortcuts_window.clone(), Group::Window, s)),
                    ]),
                ),
            )
    }
}
