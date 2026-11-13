mod start;
mod title_bar;
mod window_tabs;

use std::path::{Path, PathBuf};

use git_repo::Repo;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Render, SharedString,
    Styled as _, Subscription, Window, actions, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
    v_flex,
};
use ui_resources::Icons;

use crate::localization::tr;
use crate::recent::{RecentRepos, StoredReview, now, recent};
use crate::review::{ReviewEvent, ReviewSource, ReviewView};
use crate::settings_view::SettingsView;
use start::{StartEvent, StartView};

actions!(
    workspace,
    [
        OpenRepository,
        NewReview,
        ShowSettings,
        CloseWindow,
        NewWindow,
        ShowNextWindowTab,
        ShowPreviousWindowTab
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenRepository, None),
        KeyBinding::new("ctrl-o", OpenRepository, None),
        KeyBinding::new("cmd-n", NewReview, None),
        KeyBinding::new("ctrl-n", NewReview, None),
        KeyBinding::new("cmd-,", ShowSettings, None),
        KeyBinding::new("ctrl-,", ShowSettings, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("ctrl-w", CloseWindow, None),
        KeyBinding::new("cmd-shift-n", NewWindow, None),
        KeyBinding::new("ctrl-shift-n", NewWindow, None),
        KeyBinding::new("cmd-}", ShowNextWindowTab, None),
        KeyBinding::new("cmd-{", ShowPreviousWindowTab, None),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Start,
    Review,
    Settings,
}

pub struct Workspace {
    repo: Option<Repo>,
    repo_name: SharedString,
    screen: Screen,
    previous_screen: Screen,
    start: Option<Entity<StartView>>,
    review: Option<Entity<ReviewView>>,
    settings: Option<Entity<SettingsView>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            repo: None,
            repo_name: SharedString::default(),
            screen: Screen::Start,
            previous_screen: Screen::Start,
            start: None,
            review: None,
            settings: None,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        if let Some(path) = repo {
            this.open_repo(&path, true, window, cx);
        }
        this.update_title(window, cx);
        this
    }

    pub fn repo_path(&self) -> Option<&Path> {
        self.repo.as_ref().map(Repo::root)
    }

    pub fn open_repo(
        &mut self,
        path: &Path,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = match Repo::open(path) {
            Ok(repo) => repo,
            Err(err) => {
                log::warn!("open {}: {err}", path.display());
                window.push_notification(
                    Notification::error(err.to_string()).title(tr().open_repo_failed.clone()),
                    cx,
                );
                return;
            }
        };
        cx.global_mut::<RecentRepos>().touch(repo.root(), now());
        self.repo_name = repo.name().into();
        let start = cx.new(|cx| StartView::new(repo.clone(), window, cx));
        self._subscriptions =
            vec![
                cx.subscribe_in(&start, window, |this, _, event, window, cx| match event {
                    StartEvent::Open(source) => this.open_review(source.clone(), window, cx),
                    StartEvent::OpenSettings => this.show_settings(window, cx),
                }),
            ];
        self.start = Some(start);
        self.review = None;
        self.screen = Screen::Start;
        self.repo = Some(repo);
        if restore
            && let Some(source) = self
                .repo
                .as_ref()
                .and_then(|r| recent(cx).last_review(r.root()))
                .map(StoredReview::to_source)
        {
            self.open_review(source, window, cx);
        }
        crate::window_registry::set_repo(window, self.repo_path().map(Path::to_path_buf), cx);
        self.update_title(window, cx);
        cx.notify();
    }

    pub fn open_review(
        &mut self,
        source: ReviewSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.global_mut::<RecentRepos>()
            .set_last_review(repo.root(), StoredReview::from_source(&source));
        let review = cx.new(|cx| ReviewView::new(repo, source, window, cx));
        let subscription = cx.subscribe_in(
            &review,
            window,
            |this, _, event: &ReviewEvent, window, cx| match event {
                ReviewEvent::Changed => {
                    this.update_title(window, cx);
                    cx.notify();
                }
            },
        );
        self._subscriptions.truncate(1);
        self._subscriptions.push(subscription);
        review.update(cx, |review, cx| review.focus_tree(window, cx));
        self.review = Some(review);
        self.screen = Screen::Review;
        self.update_title(window, cx);
        cx.notify();
    }

    fn update_title(&self, window: &mut Window, cx: &mut App) {
        let title = match (&self.repo, &self.review) {
            (Some(_), Some(review)) => format!("{} — {}", self.repo_name, review.read(cx).title()),
            (Some(_), None) => self.repo_name.to_string(),
            _ => "Sloptaster".to_string(),
        };
        if window.window_title() != title {
            window.set_window_title(&title);
            gpui::SystemWindowTabController::update_tab_title(
                cx,
                window.window_handle().window_id(),
                SharedString::from(title),
            );
        }
    }

    pub fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Settings {
            self.screen = self.previous_screen;
        } else {
            if self.settings.is_none() {
                self.settings = Some(cx.new(|cx| SettingsView::new(window, cx)));
            }
            self.previous_screen = self.screen;
            self.screen = Screen::Settings;
        }
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Settings {
            self.screen = self.previous_screen;
            if self.screen == Screen::Start
                && let Some(start) = &self.start
            {
                start.update(cx, |start, cx| start.refresh(window, cx));
            }
            cx.notify();
        }
    }

    fn new_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        if let Some(start) = &self.start {
            start.update(cx, |start, cx| start.refresh(window, cx));
        }
        self.screen = Screen::Start;
        cx.notify();
    }

    fn back_to_review(&mut self, cx: &mut Context<Self>) {
        if self.review.is_some() {
            self.screen = Screen::Review;
            cx.notify();
        }
    }

    pub fn prompt_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr().open.clone()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_path(path, true, window, cx)
            });
        })
        .detach();
    }

    pub fn open_path(
        &mut self,
        path: PathBuf,
        new_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_window && self.repo.is_some() {
            crate::open_window(Some(path), cx);
        } else if self.repo_path() != Some(path.as_path()) {
            self.open_repo(&path, true, window, cx);
        }
    }

    fn render_welcome(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = tr();
        let theme = cx.theme();
        let recents: Vec<PathBuf> = recent(cx)
            .repos()
            .iter()
            .take(8)
            .map(|r| r.path.clone())
            .collect();
        let mut list = v_flex().gap_1().w(px(420.));
        for (ix, path) in recents.into_iter().enumerate() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let display = path.display().to_string();
            list = list.child(
                Button::new(("welcome-recent", ix))
                    .ghost()
                    .w_full()
                    .justify_start()
                    .icon(Icon::new(Icons::FolderGit2))
                    .label(format!("{name}    {display}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_path(path.clone(), false, window, cx)
                    })),
            );
        }
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(div().text_2xl().font_semibold().child("Sloptaster"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(s.welcome_hint.clone()),
            )
            .child(
                Button::new("welcome-open")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label(s.open_repository.clone())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.prompt_open_repository(window, cx)),
                    ),
            )
            .child(list)
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match (self.screen, &self.repo) {
            (Screen::Settings, _) => self
                .settings
                .clone()
                .map(|s| s.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            (_, None) => self.render_welcome(cx).into_any_element(),
            (Screen::Review, Some(_)) if self.review.is_some() => self
                .review
                .clone()
                .map(|r| r.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            _ => self
                .start
                .clone()
                .map(|s| s.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| {
                this.prompt_open_repository(window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewReview, window, cx| this.new_review(window, cx)))
            .on_action(
                cx.listener(|this, _: &ShowSettings, window, cx| this.show_settings(window, cx)),
            )
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &ShowNextWindowTab, window, cx| {
                window_tabs::activate_next(window, cx, 1)
            }))
            .on_action(cx.listener(|_, _: &ShowPreviousWindowTab, window, cx| {
                window_tabs::activate_next(window, cx, -1)
            }))
            .child(self.render_title_bar(window, cx))
            .child(window_tabs::render(window, cx))
            .child(div().flex_1().min_h_0().child(body))
            .when(self.screen == Screen::Settings, |el| {
                el.on_action(cx.listener(
                    |this, _: &crate::settings_view::CloseSettings, window, cx| {
                        this.close_settings(window, cx)
                    },
                ))
            })
    }
}
