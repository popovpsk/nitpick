use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement as _, SharedString, Styled as _,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ui_resources::Icons;

use super::{Screen, Workspace};
use crate::localization::tr;

impl Workspace {
    pub(super) fn render_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = tr();
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
            .tooltip(s.switch_repository_tooltip.clone());
        let repo_picker = self
            .repo_picker
            .read(cx)
            .popover("repo-picker-popover", repo_button, cx);

        let review_button = self.repo.as_ref().map(|_| {
            let (label, icon) = match &self.review {
                Some(review) => {
                    let review = review.read(cx);
                    let icon = if review.has_mr() {
                        Icons::GitPullRequest
                    } else {
                        Icons::GitCompare
                    };
                    (review.title(), icon)
                }
                None => (s.choose_review.clone(), Icons::GitCompare),
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
                        this.back_to_review(window, cx);
                    } else {
                        this.new_review(window, cx);
                    }
                }))
        });

        let theme = cx.theme();
        TitleBar::new()
            .when(window.is_fullscreen(), |bar| bar.pl(px(0.)))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .child(repo_picker)
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
                        .tooltip(s.settings.clone())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_settings(window, cx)),
                        ),
                ),
            )
    }
}

pub(super) fn repo_picker_footer(
    view: WeakEntity<Workspace>,
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    move |_, _| {
        let s = tr();
        let open_view = view.clone();
        let add_view = view.clone();
        h_flex()
            .gap_1()
            .child(
                Button::new("picker-open-repository")
                    .ghost()
                    .small()
                    .icon(IconName::FolderOpen)
                    .label(s.open_repository_ellipsis.clone())
                    .on_click(move |_, window, cx| {
                        let _ = open_view.update(cx, |this, cx| {
                            this.close_repo_picker(cx);
                            this.prompt_open_repository(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("picker-add-repositories")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label(s.add_repositories_ellipsis.clone())
                    .on_click(move |_, window, cx| {
                        let _ = add_view.update(cx, |this, cx| {
                            this.close_repo_picker(cx);
                            this.prompt_add_repositories(window, cx);
                        });
                    }),
            )
            .into_any_element()
    }
}
