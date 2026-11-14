use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _, ThemeRegistry,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use ui_resources::i18n::Lang;
use ui_resources::themes::ZedThemeFile;

use crate::localization::{sync_active_lang, tr};
use crate::settings_store::{
    DiffLayout, LangChoice, ThemeChoice, TreeSide, apply_saved_theme, apply_theme,
    end_syntax_preview, preview_syntax_colors, repo_key, settings, syntax_palette_names,
    update_settings,
};

actions!(settings_view, [CloseSettings]);

const HIDDEN_SAVE_DELAY: Duration = Duration::from_millis(400);

fn invalid_patterns(patterns: &[String]) -> Option<SharedString> {
    let (_, errors) = review_core::HideRules::new(patterns, &[]);
    (!errors.is_empty()).then(|| {
        let list: Vec<&str> = errors.iter().map(|e| e.pattern.as_str()).collect();
        tr().hidden_files_invalid(&list.join(", ")).into()
    })
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
    _save: Task<()>,
}

pub struct SettingsView {
    gitlab_url: Entity<InputState>,
    gitlab_token: Entity<InputState>,
    hidden_global: HiddenRules,
    hidden_repo: HiddenRules,
    repo: Option<(PathBuf, SharedString)>,
    _hidden_subscriptions: [Subscription; 2],
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
            gitlab_url,
            gitlab_token,
            hidden_global: HiddenRules {
                input: global_input,
                invalid: None,
                _save: Task::ready(()),
            },
            hidden_repo: HiddenRules {
                input: repo_input,
                invalid: None,
                _save: Task::ready(()),
            },
            repo: None,
            _hidden_subscriptions: subscriptions,
            status: GitlabStatus::Idle,
            zed_files: Rc::default(),
            _check: Task::ready(()),
            _zed_discovery: zed_discovery,
            _zed_import: Task::ready(()),
        };
        this.reload_hidden(window, cx);
        this
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
        self.reload_hidden(window, cx);
        cx.notify();
    }

    fn reload_hidden(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = settings(cx);
        let global = current.hidden_files.join("\n");
        let repo = self
            .repo
            .as_ref()
            .map(|(root, _)| current.repo_hidden(root).join("\n"))
            .unwrap_or_default();
        self.hidden_global.invalid = invalid_patterns(&current.hidden_files);
        self.hidden_repo.invalid = self
            .repo
            .as_ref()
            .and_then(|(root, _)| invalid_patterns(current.repo_hidden(root)));
        self.hidden_global
            .input
            .update(cx, |state, cx| state.set_value(global, window, cx));
        self.hidden_repo
            .input
            .update(cx, |state, cx| state.set_value(repo, window, cx));
    }

    fn schedule_hidden_save(&mut self, repository: bool, cx: &mut Context<Self>) {
        let rules = if repository {
            &mut self.hidden_repo
        } else {
            &mut self.hidden_global
        };
        let patterns: Vec<String> = rules
            .input
            .read(cx)
            .value()
            .lines()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.trim().is_empty())
            .collect();
        rules.invalid = invalid_patterns(&patterns);
        let root = self.repo.as_ref().map(|(root, _)| root.clone());
        let task = cx.spawn(async move |_, cx| {
            cx.background_executor().timer(HIDDEN_SAVE_DELAY).await;
            cx.update(|cx| {
                update_settings(cx, move |s| match (repository, root) {
                    (false, _) => s.hidden_files = patterns,
                    (true, Some(root)) if patterns.is_empty() => {
                        s.repo_hidden_files.remove(&repo_key(&root));
                    }
                    (true, Some(root)) => {
                        s.repo_hidden_files.insert(repo_key(&root), patterns);
                    }
                    (true, None) => {}
                })
            });
        });
        let rules = if repository {
            &mut self.hidden_repo
        } else {
            &mut self.hidden_global
        };
        rules._save = task;
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
            ));
        let review = SettingGroup::new()
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
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(CloseSettings), cx)
                            }),
                    )
                    .child(div().font_semibold().child(s.settings.clone())),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new("slopcop-settings").pages([SettingPage::new(s.settings.clone())
                        .group(appearance)
                        .group(review)
                        .group(hidden)
                        .group(gitlab)]),
                ),
            )
    }
}
