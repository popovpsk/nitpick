mod start;
mod title_bar;
mod window_tabs;

use std::path::{Path, PathBuf};

use git_repo::Repo;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString, Styled as _,
    Subscription, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};
use ui_resources::Icons;

use crate::localization::tr;
use crate::picker::{Picker, PickerEntry, PickerEvent};
use crate::recent::{RecentRepos, StoredReview, now, recent};
use crate::review::{ReviewEvent, ReviewSource, ReviewView};
use crate::settings_view::SettingsSection;
use crate::settings_window::{self, Opener};
use start::{StartEvent, StartView};

actions!(
    workspace,
    [
        OpenRepository,
        AddRepositories,
        NewReview,
        ShowSettings,
        CloseWindow,
        NewWindow,
        ShowNextWindowTab,
        ShowPreviousWindowTab
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Start,
    Review,
}

pub struct Workspace {
    repo: Option<Repo>,
    repo_name: SharedString,
    screen: Screen,
    start: Option<Entity<StartView>>,
    review: Option<Entity<ReviewView>>,
    repo_picker: Entity<Picker>,
    repo_paths: Vec<PathBuf>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _picker_subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let footer = title_bar::repo_picker_footer(cx.entity().downgrade());
        let repo_picker = cx.new(|cx| {
            Picker::new(
                Icons::FolderGit2,
                || tr().search_repositories.clone(),
                px(520.),
                window,
                cx,
            )
            .with_footer(footer)
            .with_row_action(
                Icons::ArrowUpRight,
                || tr().open_in_new_window.clone(),
                cx,
            )
        });
        let picker_subscriptions = vec![
            cx.subscribe_in(
                &repo_picker,
                window,
                |this, _, event: &PickerEvent, window, cx| {
                    if let PickerEvent::Confirm { entry, secondary } = event
                        && let Some(path) = this.repo_paths.get(*entry).cloned()
                    {
                        this.open_path(path, *secondary, window, cx);
                    }
                },
            ),
            cx.observe(&repo_picker, |_, _, cx| cx.notify()),
            cx.observe_global_in::<RecentRepos>(window, |this, window, cx| {
                this.refresh_repo_picker(window, cx)
            }),
        ];
        let mut this = Self {
            repo: None,
            repo_name: SharedString::default(),
            screen: Screen::Start,
            start: None,
            review: None,
            repo_picker,
            repo_paths: Vec::new(),
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
            _picker_subscriptions: picker_subscriptions,
        };
        if let Some(path) = repo {
            this.open_repo(&path, true, window, cx);
        }
        this.refresh_repo_picker(window, cx);
        this.update_title(window, cx);
        this
    }

    fn refresh_repo_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.repo_path();
        let home = dirs::home_dir();
        let repos = recent(cx).repos();
        let checked = repos.iter().position(|r| Some(r.path.as_path()) == current);
        let entries: Vec<PickerEntry> = repos
            .iter()
            .map(|r| PickerEntry {
                label: repo_label(&r.path),
                detail: display_path(&r.path, home.as_deref()),
                search_detail: true,
            })
            .collect();
        self.repo_paths = repos.iter().map(|r| r.path.clone()).collect();
        self.repo_picker.update(cx, |picker, cx| {
            picker.set_entries(entries, checked, window, cx)
        });
    }

    fn close_repo_picker(&mut self, cx: &mut Context<Self>) {
        self.repo_picker.update(cx, |picker, cx| picker.close(cx));
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
        self.focus_screen(window, cx);
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
        let root = self.repo_path().map(Path::to_path_buf);
        crate::window_registry::set_repo(window, root.clone(), cx);
        settings_window::repository_changed(window.window_handle().window_id(), root, cx);
        self.refresh_repo_picker(window, cx);
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
        self.close_screen_pickers(cx);
        cx.global_mut::<RecentRepos>()
            .set_last_review(repo.root(), StoredReview::from_source(&source));
        if let (Some(start), ReviewSource::Branches { target, source }) = (&self.start, &source) {
            start.update(cx, |start, cx| start.select_branches(target, source, cx));
        }
        let review = cx.new(|cx| ReviewView::new(repo, source, window, cx));
        let subscription = cx.subscribe_in(
            &review,
            window,
            |this, _, event: &ReviewEvent, window, cx| match event {
                ReviewEvent::Changed => {
                    this.update_title(window, cx);
                    cx.notify();
                }
                ReviewEvent::Open(source) => this.open_review(source.clone(), window, cx),
                ReviewEvent::OpenSettings => this.show_settings(window, cx),
                ReviewEvent::EditHiddenFiles => {
                    this.open_settings(Some(SettingsSection::HiddenFiles), window, cx)
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
            _ => "Nitpick".to_string(),
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

    fn close_screen_pickers(&self, cx: &mut Context<Self>) {
        if let Some(start) = &self.start {
            start.update(cx, |start, cx| start.close_pickers(cx));
        }
        if let Some(review) = &self.review {
            review.update(cx, |review, cx| review.close_branch_pickers(cx));
        }
    }

    pub fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(None, window, cx);
    }

    fn open_settings(
        &mut self,
        section: Option<SettingsSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_screen_pickers(cx);
        let opener = Opener::of(window, self.repo_path().map(Path::to_path_buf), cx);
        settings_window::open(Some(opener), section, cx);
    }

    fn keep_focus_inside(&self, window: &mut Window, cx: &mut Context<Self>) {
        let stray = |this: &Self, window: &mut Window, cx: &mut Context<Self>| {
            window.is_window_active()
                && window.focused(cx).is_none()
                && !this.focus.contains_focused(window, cx)
                && !window.has_active_dialog(cx)
                && !window.has_active_sheet(cx)
        };
        if stray(self, window, cx) {
            cx.defer_in(window, move |this, window, cx| {
                if stray(this, window, cx) {
                    this.focus_screen(window, cx);
                }
            });
        }
    }

    fn focus_screen(&self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.screen, &self.review) {
            (Screen::Review, Some(review)) => {
                review.update(cx, |review, cx| review.focus_main(window, cx))
            }
            _ => window.focus(&self.focus, cx),
        }
    }

    fn new_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        self.close_screen_pickers(cx);
        if let Some(start) = &self.start {
            start.update(cx, |start, cx| start.refresh(window, cx));
        }
        self.screen = Screen::Start;
        self.focus_screen(window, cx);
        cx.notify();
    }

    fn back_to_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.is_some() {
            self.close_screen_pickers(cx);
            self.screen = Screen::Review;
            self.focus_screen(window, cx);
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

    pub fn prompt_add_repositories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let scanned = folder.clone();
            let result = cx
                .background_spawn(async move { git_repo::find_repositories(&scanned) })
                .await;
            let _ = this.update_in(cx, |_, window, cx| {
                let s = tr();
                let notification = match result {
                    Ok(found) => match cx.global_mut::<RecentRepos>().add(&found) {
                        0 => Notification::info(s.no_new_repositories.clone()),
                        added => Notification::success(s.repositories_added(added)),
                    },
                    Err(err) => {
                        log::warn!("scan {}: {err}", folder.display());
                        Notification::error(err.to_string()).title(s.scan_failed.clone())
                    }
                };
                window.push_notification(notification, cx);
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
            .child(div().text_2xl().font_semibold().child("Nitpick"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(s.welcome_hint.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("welcome-open")
                            .primary()
                            .icon(IconName::FolderOpen)
                            .label(s.open_repository.clone())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_open_repository(window, cx)
                            })),
                    )
                    .child(
                        Button::new("welcome-add-repositories")
                            .outline()
                            .icon(IconName::Plus)
                            .label(s.add_repositories_ellipsis.clone())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_add_repositories(window, cx)
                            })),
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
        self.keep_focus_inside(window, cx);
        let body = match (self.screen, &self.repo) {
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
            .on_action(cx.listener(|this, _: &AddRepositories, window, cx| {
                this.prompt_add_repositories(window, cx)
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
    }
}

fn repo_label(path: &Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
        .into()
}

fn display_path(path: &Path, home: Option<&Path>) -> SharedString {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()).into()
        }
        _ => path.display().to_string().into(),
    }
}
