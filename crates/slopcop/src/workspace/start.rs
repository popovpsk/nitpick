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
    v_flex,
};
use ui_resources::Icons;

use crate::localization::tr;
use crate::picker::{Picker, PickerEvent, branch_entries};
use crate::review::ReviewSource;
use crate::settings_store::settings;

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
    name: Option<String>,
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
        let known = |name: Option<&str>| {
            name.filter(|name| branches.iter().any(|b| b.name == *name))
                .map(str::to_string)
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

    pub fn select_branches(&mut self, target: &str, source: &str, cx: &mut Context<Self>) {
        self.target.name = Some(target.to_string());
        self.source.name = Some(source.to_string());
        self.sync_selection(cx);
    }

    fn on_pick(&mut self, side: Side, event: &PickerEvent, cx: &mut Context<Self>) {
        let PickerEvent::Confirm { entry, .. } = event else {
            return;
        };
        let Some(branch) = self.branches.get(*entry) else {
            return;
        };
        let name = Some(branch.name.clone());
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
        if let (Some(target), Some(source)) = (&self.target.name, &self.source.name)
            && target != source
        {
            cx.emit(StartEvent::Open(ReviewSource::Branches {
                target: target.clone(),
                source: source.clone(),
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
                Some(name) => (SharedString::from(name.clone()), theme.foreground),
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
