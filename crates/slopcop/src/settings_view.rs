use gpui::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _, ThemeRegistry,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use ui_resources::i18n::Lang;

use crate::localization::{sync_active_lang, tr};
use crate::settings_store::{
    DiffLayout, LangChoice, ThemeChoice, TreeSide, apply_saved_theme, apply_theme, settings,
    update_settings,
};

actions!(settings_view, [CloseSettings]);

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

pub struct SettingsView {
    gitlab_url: Entity<InputState>,
    gitlab_token: Entity<InputState>,
    status: GitlabStatus,
    _check: Task<()>,
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
        Self {
            gitlab_url,
            gitlab_token,
            status: GitlabStatus::Idle,
            _check: Task::ready(()),
        }
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
        let appearance = SettingGroup::new()
            .title(s.appearance.clone())
            .item(
                SettingItem::new(
                    s.theme.clone(),
                    SettingField::render(|_, _, cx| theme_dropdown(cx)),
                )
                .description(s.theme_desc.clone()),
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
                        .group(gitlab)]),
                ),
            )
    }
}
