use std::path::PathBuf;

use gpui::{
    Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
};
use ui_resources::Icons;

use super::{Screen, Workspace};
use crate::localization::tr;
use crate::recent::recent;

impl Workspace {
    pub(super) fn render_title_bar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = tr();
        let theme = cx.theme();
        let view = cx.entity().downgrade();
        let repo_label: SharedString = if self.repo.is_some() {
            self.repo_name.clone()
        } else {
            s.open_repository.clone()
        };
        let repo_button = Button::new("repo-picker")
            .ghost()
            .small()
            .icon(Icon::new(Icons::FolderGit2))
            .label(repo_label)
            .dropdown_caret(true)
            .tooltip(s.recent_repositories.clone())
            .dropdown_menu(move |menu, _, cx| {
                let current = view
                    .upgrade()
                    .and_then(|v| v.read(cx).repo_path().map(|p| p.to_path_buf()));
                let entries: Vec<PathBuf> =
                    recent(cx).repos().iter().map(|r| r.path.clone()).collect();
                let mut menu = menu.min_w(px(320.)).max_h(px(480.)).scrollable(true);
                menu = menu.label(tr().recent_repositories_hint.clone());
                for path in entries {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let checked = current.as_deref() == Some(path.as_path());
                    let view = view.clone();
                    let target = path.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{name}  —  {}", path.display()))
                            .checked(checked)
                            .on_click(move |event, window, cx| {
                                let new_window = event.modifiers().secondary(cx);
                                let target = target.clone();
                                if let Some(view) = view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.open_path(target, new_window, window, cx)
                                    });
                                }
                            }),
                    );
                }
                let open_view = view.clone();
                menu.separator().item(
                    PopupMenuItem::new(tr().open_repository_ellipsis.clone())
                        .icon(IconName::FolderOpen)
                        .on_click(move |_, window, cx| {
                            if let Some(view) = open_view.upgrade() {
                                view.update(cx, |this, cx| this.prompt_open_repository(window, cx));
                            }
                        }),
                )
            });

        let review_button = self.repo.as_ref().map(|_| {
            let (label, icon) = match (&self.review, self.screen) {
                (Some(review), _) => {
                    let review = review.read(cx);
                    let (viewed, total) = review.progress();
                    let icon = if review.has_mr() {
                        Icons::GitPullRequest
                    } else {
                        Icons::GitCompare
                    };
                    (
                        SharedString::from(format!("{}  {viewed}/{total}", review.title())),
                        icon,
                    )
                }
                (None, _) => (s.choose_review.clone(), Icons::GitCompare),
            };
            Button::new("review-picker")
                .ghost()
                .small()
                .icon(Icon::new(icon))
                .label(label)
                .tooltip(s.choose_review_tooltip.clone())
                .selected(self.screen == Screen::Start)
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.screen == Screen::Start {
                        this.back_to_review(cx);
                    } else {
                        this.new_review(window, cx);
                    }
                }))
        });

        TitleBar::new()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .child(repo_button)
                    .when_some(review_button, |el, button| {
                        el.child(div().text_color(theme.muted_foreground).child("/"))
                            .child(div().min_w_0().overflow_hidden().child(button))
                    }),
            )
            .child(
                h_flex().pr_2().gap_1().child(
                    Button::new("open-settings")
                        .ghost()
                        .small()
                        .icon(IconName::Settings)
                        .selected(self.screen == Screen::Settings)
                        .tooltip(s.settings.clone())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_settings(window, cx)),
                        ),
                ),
            )
    }
}
