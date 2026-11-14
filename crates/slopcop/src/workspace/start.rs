use git_repo::{Branch, BranchKind, Repo};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use ui_resources::Icons;

use crate::localization::tr;
use crate::review::ReviewSource;
use crate::settings_store::settings;

pub enum StartEvent {
    Open(ReviewSource),
    OpenSettings,
}

impl EventEmitter<StartEvent> for StartView {}

type BranchSelect = Entity<SelectState<SearchableVec<SharedString>>>;

pub struct StartView {
    repo: Repo,
    branches: Vec<Branch>,
    loading_branches: bool,
    target: BranchSelect,
    source: BranchSelect,
    mr_input: Entity<InputState>,
    filter: Entity<InputState>,
    gitlab_project: Option<String>,
    merge_requests: Vec<MrRow>,
    filter_text: String,
    mr_state: MrListState,
    _branches_task: Task<()>,
    _mrs_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

struct MrRow {
    iid: u64,
    title: SharedString,
    subtitle: SharedString,
    key: String,
    draft: bool,
    source: ReviewSource,
}

impl MrRow {
    fn new(mr: &gitlab::MergeRequest, project: &str) -> Self {
        let subtitle = format!(
            "!{} · {} · {} → {}",
            mr.iid, mr.author.name, mr.source_branch, mr.target_branch
        );
        let key = format!("{} {} {}", mr.title, subtitle, mr.author.username).to_lowercase();
        Self {
            iid: mr.iid,
            title: mr.title.clone().into(),
            subtitle: subtitle.into(),
            key,
            draft: mr.draft,
            source: ReviewSource::MergeRequest {
                project: project.to_string(),
                iid: mr.iid,
            },
        }
    }
}

enum MrListState {
    Unconfigured,
    NoRemote,
    Loading,
    Loaded,
    Failed(SharedString),
}

impl StartView {
    pub fn new(repo: Repo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = tr();
        let target = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let source = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
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
            cx.subscribe(
                &target,
                |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
            ),
            cx.subscribe(
                &source,
                |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
            ),
        ];
        let mut this = Self {
            repo,
            branches: Vec::new(),
            loading_branches: true,
            target,
            source,
            mr_input,
            filter,
            gitlab_project: None,
            merge_requests: Vec::new(),
            filter_text: String::new(),
            mr_state: MrListState::Unconfigured,
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
                    Err(err) => log::warn!("branches: {err}"),
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
        let names: Vec<SharedString> = branches
            .iter()
            .map(|b| SharedString::from(b.name.clone()))
            .collect();
        let pick = |name: Option<&str>| -> Option<IndexPath> {
            let name = name?;
            names
                .iter()
                .position(|n| n.as_ref() == name)
                .map(IndexPath::new)
        };
        let prev_target = self.target.read(cx).selected_value().cloned();
        let prev_source = self.source.read(cx).selected_value().cloned();
        let target_ix = pick(prev_target.as_deref()).or_else(|| pick(default.as_deref()));
        let source_ix = pick(prev_source.as_deref())
            .or_else(|| pick(current.as_deref()).filter(|ix| Some(*ix) != target_ix));
        for (select, ix) in [(&self.target, target_ix), (&self.source, source_ix)] {
            let items = SearchableVec::new(names.clone());
            select.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_index(ix, window, cx);
            });
        }
        self.branches = branches;
    }

    fn load_merge_requests(&mut self, cx: &mut Context<Self>) {
        let Some(client) = settings(cx).gitlab_client() else {
            self.mr_state = MrListState::Unconfigured;
            self._mrs_task = Task::ready(());
            return;
        };
        self.mr_state = MrListState::Loading;
        let repo = self.repo.clone();
        let task = cx.spawn(async move |this, cx| {
            let base_url = client.base_url().to_string();
            let project = cx
                .background_spawn(async move {
                    crate::review::model::gitlab_remote(&repo, &base_url, None)
                })
                .await
                .map(|(_, path)| path);
            let Some(project) = project else {
                let _ = this.update(cx, |this, cx| {
                    this.mr_state = MrListState::NoRemote;
                    cx.notify();
                });
                return;
            };
            let query_project = project.clone();
            let result = net_runtime::spawn(async move {
                client
                    .merge_requests(
                        &gitlab::ProjectRef::Path(query_project),
                        &gitlab::MrQuery::default(),
                    )
                    .await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        this.merge_requests =
                            list.iter().map(|mr| MrRow::new(mr, &project)).collect();
                        this.mr_state = MrListState::Loaded;
                    }
                    Err(err) => this.mr_state = MrListState::Failed(err.to_string().into()),
                }
                this.gitlab_project = Some(project);
                cx.notify();
            });
        });
        self._mrs_task = task;
    }

    fn compare(&mut self, cx: &mut Context<Self>) {
        let target = self.target.read(cx).selected_value().cloned();
        let source = self.source.read(cx).selected_value().cloned();
        if let (Some(target), Some(source)) = (target, source)
            && target != source
        {
            cx.emit(StartEvent::Open(ReviewSource::Branches {
                target: target.to_string(),
                source: source.to_string(),
            }));
        }
    }

    fn swap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.target.read(cx).selected_index(cx);
        let source = self.source.read(cx).selected_index(cx);
        self.target
            .update(cx, |s, cx| s.set_selected_index(source, window, cx));
        self.source
            .update(cx, |s, cx| s.set_selected_index(target, window, cx));
        cx.notify();
    }

    fn open_mr_from_input(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.mr_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let base = settings(cx).gitlab_url.trim().to_string();
        if let Some(url) = gitlab::parse_mr_url_with_base(&text, &base) {
            cx.emit(StartEvent::Open(ReviewSource::MergeRequest {
                project: url.project_path,
                iid: url.iid,
            }));
        } else if let (Some(iid), Some(project)) = (
            gitlab::parse_mr_reference(&text),
            self.gitlab_project.clone(),
        ) {
            cx.emit(StartEvent::Open(ReviewSource::MergeRequest {
                project,
                iid,
            }));
        }
    }

    fn branch_hint(&self, name: Option<&SharedString>) -> Option<SharedString> {
        let name = name?;
        let branch = self.branches.iter().find(|b| b.name == name.as_ref())?;
        let kind = match branch.kind {
            BranchKind::Local => "",
            BranchKind::Remote => "remote · ",
        };
        let subject: String = branch.subject.chars().take(60).collect();
        Some(
            format!(
                "{kind}{} · {subject}",
                &branch.sha[..branch.sha.len().min(8)]
            )
            .into(),
        )
    }

    fn render_mr_list(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let s = tr();
        let theme = cx.theme();
        let body = match &self.mr_state {
            MrListState::Unconfigured => v_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(s.gitlab_not_configured.clone()),
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
                .child(s.no_gitlab_remote.clone())
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
                            .child(
                                Icon::new(Icons::GitPullRequest)
                                    .small()
                                    .text_color(theme.green),
                            )
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
                                            .child(mr.subtitle.clone()),
                                    ),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(StartEvent::Open(source.clone()))
                            })),
                    );
                }
                if shown == 0 {
                    list = list.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(s.no_mrs.clone()),
                    );
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
        let target = self.target.read(cx).selected_value().cloned();
        let source = self.source.read(cx).selected_value().cloned();
        let can_compare = matches!((&target, &source), (Some(t), Some(s)) if t != s);
        let target_hint = self.branch_hint(target.as_ref());
        let source_hint = self.branch_hint(source.as_ref());
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
        let branch_field = |label: SharedString,
                            select: &BranchSelect,
                            hint: Option<SharedString>,
                            id: &'static str| {
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
                .child(
                    Select::new(select)
                        .id(id)
                        .icon(Icon::new(Icons::GitBranch))
                        .placeholder(s.choose_branch.clone())
                        .search_placeholder(s.search.clone())
                        .menu_max_h(px(360.)),
                )
                .child(
                    div()
                        .h(px(16.))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .children(hint),
                )
        };
        let compare = card(s.compare_branches.clone(), Icon::new(Icons::GitCompare))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(s.compare_hint.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(branch_field(
                        s.target_branch.clone(),
                        &self.target,
                        target_hint,
                        "target-select",
                    ))
                    .child(
                        div().pt(px(20.)).child(
                            Button::new("swap-branches")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .tooltip(s.swap.clone())
                                .on_click(cx.listener(|this, _, window, cx| this.swap(window, cx))),
                        ),
                    )
                    .child(branch_field(
                        s.source_branch.clone(),
                        &self.source,
                        source_hint,
                        "source-select",
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
                            .disabled(!can_compare)
                            .on_click(cx.listener(|this, _, _, cx| this.compare(cx))),
                    ),
            );
        let mrs = card(s.merge_requests.clone(), Icon::new(Icons::GitPullRequest)).child(mr_list);
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
                    .child(mrs),
            )
    }
}
