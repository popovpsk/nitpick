use forge::ForgeKind;
use git_repo::{Branch, BranchKind, Repo};
use gpui::{
    AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    tag::Tag,
    tooltip::Tooltip,
    v_flex,
};
use ui_resources::Icons;

use crate::localization::tr;
use crate::picker::{Picker, PickerEvent, branch_entries};
use crate::review::ReviewSource;
use crate::review::avatars::{self, Face};
use crate::settings_store::{SettingsStore, settings};

pub enum StartEvent {
    Open(ReviewSource),
    OpenSettings,
}

impl EventEmitter<StartEvent> for StartView {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Target,
    Source,
}

struct BranchField {
    name: Option<SharedString>,
    hint: Option<SharedString>,
    picker: Entity<Picker>,
}

pub struct StartView {
    repo: Repo,
    branches: Vec<Branch>,
    loading_branches: bool,
    target: BranchField,
    source: BranchField,
    mr_input: Entity<InputState>,
    filter: Entity<InputState>,
    forge_project: Option<(ForgeKind, String)>,
    forge_key: u64,
    merge_requests: Vec<MrRow>,
    filter_text: String,
    mr_state: MrListState,
    _branches_task: Task<()>,
    _mrs_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

const MAX_FACES: usize = 4;

struct MrRow {
    iid: u64,
    title: SharedString,
    subtitle: SharedString,
    key: String,
    draft: bool,
    author: Face,
    people: Vec<Face>,
    people_tooltip: Option<SharedString>,
    comments: Option<SharedString>,
    source: ReviewSource,
}

impl MrRow {
    fn new(request: &forge::Request, project: &str, now: i64) -> Self {
        let mut subtitle = format!(
            "{} · {} · {} → {}",
            request.reference(),
            request.author.name,
            request.source_branch,
            request.target_branch
        );
        if let Some(updated) = request
            .updated_at
            .as_deref()
            .and_then(review_core::parse_timestamp)
        {
            subtitle.push_str(" · ");
            subtitle.push_str(&tr().relative_time(now - updated));
        }
        let mut people: Vec<&forge::User> = Vec::new();
        for user in request.reviewers.iter().chain(&request.assignees) {
            if user.username != request.author.username
                && !people.iter().any(|p| p.username == user.username)
            {
                people.push(user);
            }
        }
        let mut key = format!("{} {} {}", request.title, subtitle, request.author.username);
        for user in &people {
            key.push(' ');
            key.push_str(&user.name);
            key.push(' ');
            key.push_str(&user.username);
        }
        let people_tooltip = (!people.is_empty()).then(|| {
            let names: Vec<&str> = people.iter().map(|user| user.name.as_str()).collect();
            SharedString::from(names.join(", "))
        });
        Self {
            iid: request.number,
            title: request.title.clone().into(),
            subtitle: subtitle.into(),
            key: key.to_lowercase(),
            draft: request.draft,
            author: Face::of(&request.author),
            people: people
                .iter()
                .take(MAX_FACES)
                .map(|user| Face::of(user))
                .collect(),
            people_tooltip,
            comments: request
                .comments
                .filter(|&count| count > 0)
                .map(|count| count.to_string().into()),
            source: ReviewSource::MergeRequest {
                forge: request.kind,
                project: project.to_string(),
                number: request.number,
            },
        }
    }
}

enum MrListState {
    Unconfigured(ForgeKind),
    NoRemote,
    Loading,
    Loaded,
    Failed(SharedString),
}

impl StartView {
    pub fn new(repo: Repo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = tr();
        let new_picker = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                Picker::new(
                    Icons::GitBranch,
                    || tr().search_branches.clone(),
                    px(420.),
                    window,
                    cx,
                )
            })
        };
        let target = new_picker(window, cx);
        let source = new_picker(window, cx);
        let mr_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(s.mr_input_placeholder.clone()));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(s.filter_mrs.clone()));
        let subscriptions = vec![
            cx.subscribe_in(
                &mr_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.open_mr_from_input(window, cx);
                    }
                },
            ),
            cx.subscribe(&filter, |this, filter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_text = filter.read(cx).value().trim().to_lowercase();
                    cx.notify();
                }
            }),
            cx.subscribe(&target, |this, _, event: &PickerEvent, cx| {
                this.on_pick(Side::Target, event, cx)
            }),
            cx.subscribe(&source, |this, _, event: &PickerEvent, cx| {
                this.on_pick(Side::Source, event, cx)
            }),
            cx.observe(&target, |_, _, cx| cx.notify()),
            cx.observe(&source, |_, _, cx| cx.notify()),
            cx.observe_global::<SettingsStore>(|this, cx| {
                if forge_key(cx) != this.forge_key {
                    this.load_merge_requests(cx);
                }
            }),
        ];
        let mut this = Self {
            repo,
            branches: Vec::new(),
            loading_branches: true,
            target: BranchField {
                name: None,
                hint: None,
                picker: target,
            },
            source: BranchField {
                name: None,
                hint: None,
                picker: source,
            },
            mr_input,
            filter,
            forge_project: None,
            forge_key: 0,
            merge_requests: Vec::new(),
            filter_text: String::new(),
            mr_state: MrListState::Loading,
            _branches_task: Task::ready(()),
            _mrs_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.load_branches(window, cx);
        this.load_merge_requests(cx);
        this
    }

    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_branches(window, cx);
        self.load_merge_requests(cx);
    }

    fn load_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self.loading_branches = true;
        self.set_pickers_loading(true, cx);
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let branches = repo.branches()?;
                    let current = repo.current_branch()?;
                    let default = repo.default_target(&branches);
                    anyhow::Ok((branches, current, default))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading_branches = false;
                match result {
                    Ok((branches, current, default)) => {
                        this.install_branches(branches, current, default, window, cx)
                    }
                    Err(err) => {
                        log::warn!("branches: {err}");
                        this.set_pickers_loading(false, cx);
                    }
                }
                cx.notify();
            });
        });
        self._branches_task = task;
    }

    fn install_branches(
        &mut self,
        branches: Vec<Branch>,
        current: Option<String>,
        default: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let known = |name: Option<&str>| {
            name.filter(|name| branches.iter().any(|b| b.name == *name))
                .map(|name| SharedString::from(name.to_string()))
        };
        let target = known(self.target.name.as_deref()).or_else(|| known(default.as_deref()));
        let source = known(self.source.name.as_deref())
            .or_else(|| known(current.as_deref()).filter(|s| Some(s) != target.as_ref()));
        let entries = branch_entries(&branches);
        self.branches = branches;
        self.target.name = target;
        self.source.name = source;
        for picker in [&self.target.picker, &self.source.picker] {
            let entries = entries.clone();
            picker.update(cx, |picker, cx| {
                picker.set_entries(entries, None, window, cx)
            });
        }
        self.sync_selection(cx);
    }

    fn set_pickers_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        for picker in [&self.target.picker, &self.source.picker] {
            picker.update(cx, |picker, cx| {
                let loading = loading && picker.is_empty(cx);
                picker.set_loading(loading, cx);
            });
        }
    }

    pub fn close_pickers(&mut self, cx: &mut Context<Self>) {
        for picker in [&self.target.picker, &self.source.picker] {
            picker.update(cx, |picker, cx| picker.close(cx));
        }
    }

    pub fn select_branches(&mut self, target: &str, source: &str, cx: &mut Context<Self>) {
        self.target.name = Some(SharedString::from(target.to_string()));
        self.source.name = Some(SharedString::from(source.to_string()));
        self.sync_selection(cx);
    }

    fn on_pick(&mut self, side: Side, event: &PickerEvent, cx: &mut Context<Self>) {
        let PickerEvent::Confirm { entry, .. } = event else {
            return;
        };
        let Some(branch) = self.branches.get(*entry) else {
            return;
        };
        let name = Some(SharedString::from(branch.name.clone()));
        match side {
            Side::Target => self.target.name = name,
            Side::Source => self.source.name = name,
        }
        self.sync_selection(cx);
    }

    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        for field in [&mut self.target, &mut self.source] {
            let index = field
                .name
                .as_deref()
                .and_then(|name| self.branches.iter().position(|b| b.name == name));
            field.hint = index.map(|ix| branch_hint(&self.branches[ix]));
            field
                .picker
                .update(cx, |picker, cx| picker.set_checked(index, cx));
        }
        cx.notify();
    }

    fn load_merge_requests(&mut self, cx: &mut Context<Self>) {
        self.forge_key = forge_key(cx);
        let forges = settings(cx).forge_hosts();
        let connected: Vec<(ForgeKind, String)> = forges
            .iter()
            .filter(|(kind, _)| settings(cx).forge_configured(*kind))
            .cloned()
            .collect();
        let any_ready = !connected.is_empty();
        self.mr_state = MrListState::Loading;
        let repo = self.repo.clone();
        let task = cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    crate::review::model::forge_remote(&repo, &connected, None)
                        .or_else(|| crate::review::model::forge_remote(&repo, &forges, None))
                })
                .await;
            let Some(found) = found else {
                let _ = this.update(cx, |this, cx| {
                    this.forge_project = None;
                    this.mr_state = if any_ready {
                        MrListState::NoRemote
                    } else {
                        MrListState::Unconfigured(ForgeKind::GitLab)
                    };
                    cx.notify();
                });
                return;
            };
            let client = this
                .update(cx, |this, cx| {
                    this.forge_project = Some((found.kind, found.project.clone()));
                    let client = settings(cx).forge_client(found.kind);
                    if client.is_none() {
                        this.mr_state = MrListState::Unconfigured(found.kind);
                    }
                    cx.notify();
                    client
                })
                .ok()
                .flatten();
            let Some(client) = client else {
                return;
            };
            let project = found.project;
            let query_project = project.clone();
            let result =
                net_runtime::spawn(async move { client.open_requests(&query_project).await }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_secs() as i64);
                        this.merge_requests = list
                            .iter()
                            .map(|request| MrRow::new(request, &project, now))
                            .collect();
                        let urls: Vec<SharedString> = this
                            .merge_requests
                            .iter()
                            .flat_map(|row| {
                                std::iter::once(&row.author)
                                    .chain(&row.people)
                                    .filter_map(|face| face.avatar.clone())
                            })
                            .collect();
                        avatars::request(urls, cx);
                        this.mr_state = MrListState::Loaded;
                    }
                    Err(err) => this.mr_state = MrListState::Failed(err.to_string().into()),
                }
                cx.notify();
            });
        });
        self._mrs_task = task;
    }

    fn forge_kind(&self) -> ForgeKind {
        self.forge_project
            .as_ref()
            .map_or(ForgeKind::GitLab, |(kind, _)| *kind)
    }

    fn compare(&mut self, cx: &mut Context<Self>) {
        if let (Some(target), Some(source)) = (&self.target.name, &self.source.name)
            && target != source
        {
            cx.emit(StartEvent::Open(ReviewSource::Branches {
                target: target.to_string(),
                source: source.to_string(),
            }));
        }
    }

    fn swap(&mut self, cx: &mut Context<Self>) {
        std::mem::swap(&mut self.target.name, &mut self.source.name);
        self.sync_selection(cx);
    }

    fn open_mr_from_input(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.mr_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let gitlab_base = settings(cx).forge_url(ForgeKind::GitLab);
        let github_base = settings(cx).forge_url(ForgeKind::GitHub);
        if let Some(url) = forge::parse_request_url(&text, &gitlab_base, &github_base) {
            cx.emit(StartEvent::Open(ReviewSource::MergeRequest {
                forge: url.kind,
                project: url.project,
                number: url.number,
            }));
        } else if let (Some(number), Some((forge, project))) =
            (forge::parse_reference(&text), self.forge_project.clone())
        {
            cx.emit(StartEvent::Open(ReviewSource::MergeRequest {
                forge,
                project,
                number,
            }));
        }
    }

    fn render_mr_list(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let s = tr();
        let theme = cx.theme();
        let body = match &self.mr_state {
            MrListState::Unconfigured(kind) => v_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(match kind {
                            ForgeKind::GitLab => s.gitlab_not_configured.clone(),
                            ForgeKind::GitHub => s.github_not_configured.clone(),
                        }),
                )
                .child(
                    Button::new("open-settings")
                        .outline()
                        .small()
                        .icon(IconName::Settings)
                        .label(s.settings.clone())
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(StartEvent::OpenSettings))),
                )
                .into_any_element(),
            MrListState::NoRemote => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(s.no_forge_remote.clone())
                .into_any_element(),
            MrListState::Loading => h_flex()
                .gap_2()
                .child(Spinner::new().small())
                .into_any_element(),
            MrListState::Failed(message) => div()
                .text_sm()
                .text_color(theme.danger)
                .child(message.clone())
                .into_any_element(),
            MrListState::Loaded => {
                let mut list = v_flex().gap_1();
                let mut shown = 0;
                for mr in &self.merge_requests {
                    if !self.filter_text.is_empty() && !mr.key.contains(&self.filter_text) {
                        continue;
                    }
                    shown += 1;
                    let source = mr.source.clone();
                    list = list.child(
                        h_flex()
                            .id(("mr-row", mr.iid))
                            .gap_2()
                            .px_2()
                            .py_1p5()
                            .rounded(theme.radius)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.list_hover))
                            .child(mr.author.render(px(24.), cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_semibold()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .whitespace_nowrap()
                                                    .child(mr.title.clone()),
                                            )
                                            .when(mr.draft, |el| {
                                                el.child(Tag::secondary().small().child("Draft"))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(mr.subtitle.clone()),
                                    ),
                            )
                            .when(!mr.people.is_empty(), |el| {
                                let tooltip = mr.people_tooltip.clone().unwrap_or_default();
                                el.child(
                                    h_flex()
                                        .id(("mr-people", mr.iid))
                                        .flex_none()
                                        .children(mr.people.iter().enumerate().map(|(ix, face)| {
                                            div()
                                                .when(ix > 0, |el| el.ml(px(-6.)))
                                                .rounded_full()
                                                .border_2()
                                                .border_color(theme.background)
                                                .child(face.render(px(20.), cx))
                                        }))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(tooltip.clone()).build(window, cx)
                                        }),
                                )
                            })
                            .when_some(mr.comments.clone(), |el, comments| {
                                el.child(
                                    h_flex()
                                        .flex_none()
                                        .gap_0p5()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(Icon::new(Icons::MessageSquare).xsmall())
                                        .child(comments),
                                )
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(StartEvent::Open(source.clone()))
                            })),
                    );
                }
                if shown == 0 {
                    list = list.child(div().text_sm().text_color(theme.muted_foreground).child(
                        match self.forge_kind() {
                            ForgeKind::GitLab => s.no_mrs.clone(),
                            ForgeKind::GitHub => s.no_prs.clone(),
                        },
                    ));
                }
                list.into_any_element()
            }
        };
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.mr_input).flex_1())
                    .child(
                        Button::new("open-mr-input")
                            .primary()
                            .label(s.open.clone())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_mr_from_input(window, cx)
                            })),
                    ),
            )
            .when(matches!(self.mr_state, MrListState::Loaded), |el| {
                el.child(Input::new(&self.filter).small().cleanable(true))
            })
            .child(body)
            .into_any_element()
    }
}

impl Render for StartView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = tr();
        let mr_list = self.render_mr_list(cx);
        let theme = cx.theme();
        let can_compare = matches!(
            (&self.target.name, &self.source.name),
            (Some(t), Some(s)) if t != s
        );
        let card = |title: SharedString, icon: Icon| {
            v_flex()
                .w_full()
                .max_w(px(760.))
                .gap_3()
                .p_4()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(icon.text_color(theme.primary))
                        .child(div().font_semibold().child(title)),
                )
        };
        let branch_field = |label: SharedString, field: &BranchField, id: &'static str| {
            let (name, color) = match &field.name {
                Some(name) => (name.clone(), theme.foreground),
                None => (s.choose_branch.clone(), theme.muted_foreground),
            };
            let trigger = Button::new(id).outline().w_full().child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(Icons::GitBranch)
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(color)
                            .child(name),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(theme.muted_foreground),
                    ),
            );
            let picker = field
                .picker
                .read(cx)
                .popover(ElementId::from((id, 1usize)), trigger, cx)
                .trigger_style(StyleRefinement::default().w_full());
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(picker)
                .child(
                    div()
                        .h(px(16.))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .children(field.hint.clone()),
                )
        };
        let compare = card(s.compare_branches.clone(), Icon::new(Icons::GitCompare))
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(branch_field(
                        s.target_branch.clone(),
                        &self.target,
                        "target-branch",
                    ))
                    .child(
                        div().pt(px(20.)).child(
                            Button::new("swap-branches")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .tooltip(s.swap.clone())
                                .on_click(cx.listener(|this, _, _, cx| this.swap(cx))),
                        ),
                    )
                    .child(branch_field(
                        s.source_branch.clone(),
                        &self.source,
                        "source-branch",
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when(self.loading_branches, |el| el.child(Spinner::new().small()))
                    .child(
                        Button::new("compare")
                            .primary()
                            .icon(Icon::new(Icons::GitCompare))
                            .label(s.compare.clone())
                            .tooltip(s.compare_tooltip.clone())
                            .disabled(!can_compare)
                            .on_click(cx.listener(|this, _, _, cx| this.compare(cx))),
                    ),
            );
        let local = h_flex()
            .w_full()
            .max_w(px(760.))
            .gap_3()
            .px_4()
            .py_3()
            .items_center()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(Icon::new(Icons::FilePen).text_color(theme.primary))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_semibold().child(s.uncommitted_changes.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(s.uncommitted_changes_desc.clone()),
                    ),
            )
            .child(
                Button::new("review-uncommitted")
                    .outline()
                    .label(s.open.clone())
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(StartEvent::Open(ReviewSource::WorkingTree))
                    })),
            );
        let requests_title = match self.forge_kind() {
            ForgeKind::GitLab => s.merge_requests.clone(),
            ForgeKind::GitHub => s.pull_requests.clone(),
        };
        let mrs = card(requests_title, Icon::new(Icons::GitPullRequest)).child(mr_list);
        div()
            .id("start-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(theme.muted.opacity(0.3))
            .child(
                v_flex()
                    .w_full()
                    .items_center()
                    .gap_4()
                    .p_6()
                    .child(compare)
                    .child(local)
                    .child(mrs),
            )
    }
}

fn branch_hint(branch: &Branch) -> SharedString {
    let kind = match branch.kind {
        BranchKind::Local => "",
        BranchKind::Remote => "remote · ",
    };
    let subject: String = branch.subject.chars().take(60).collect();
    format!(
        "{kind}{} · {subject}",
        &branch.sha[..branch.sha.len().min(8)]
    )
    .into()
}

fn forge_key(cx: &gpui::App) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let current = settings(cx);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for kind in [ForgeKind::GitLab, ForgeKind::GitHub] {
        current.forge_url(kind).hash(&mut hasher);
        current.forge_token(kind).hash(&mut hasher);
    }
    hasher.finish()
}
