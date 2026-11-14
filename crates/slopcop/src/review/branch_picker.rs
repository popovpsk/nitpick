use git_repo::Branch;
use gpui::{AppContext as _, Context, Entity, Subscription, Task, Window, px};
use ui_resources::Icons;

use super::{PullSide, ReviewEvent, ReviewSource, ReviewView};
use crate::localization::tr;
use crate::picker::{Picker, PickerEvent, branch_entries};

pub struct BranchPickers {
    target: Entity<Picker>,
    source: Entity<Picker>,
    names: Vec<String>,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl BranchPickers {
    pub fn new(window: &mut Window, cx: &mut Context<ReviewView>) -> Self {
        let new_picker = |window: &mut Window, cx: &mut Context<ReviewView>| {
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
        let subscriptions = vec![
            cx.subscribe_in(&target, window, |this, _, event, window, cx| {
                this.on_branch_picker(PullSide::Target, event, window, cx)
            }),
            cx.subscribe_in(&source, window, |this, _, event, window, cx| {
                this.on_branch_picker(PullSide::Source, event, window, cx)
            }),
            cx.observe(&target, |_, _, cx| cx.notify()),
            cx.observe(&source, |_, _, cx| cx.notify()),
        ];
        Self {
            target,
            source,
            names: Vec::new(),
            _load: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    pub fn picker(&self, side: PullSide) -> &Entity<Picker> {
        match side {
            PullSide::Target => &self.target,
            PullSide::Source => &self.source,
        }
    }
}

impl ReviewView {
    fn on_branch_picker(
        &mut self,
        side: PullSide,
        event: &PickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PickerEvent::Open => self.load_branch_choices(window, cx),
            PickerEvent::Confirm { entry, .. } => self.pick_branch(side, *entry, cx),
        }
    }

    fn load_branch_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pickers = [
            self.branch_pickers.target.clone(),
            self.branch_pickers.source.clone(),
        ];
        for picker in &pickers {
            picker.update(cx, |picker, cx| {
                let empty = picker.is_empty(cx);
                picker.set_loading(empty, cx);
            });
        }
        let repo = self.repo.clone();
        self.branch_pickers._load = cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { repo.branches() }).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(branches) => this.install_branch_choices(branches, window, cx),
                Err(err) => {
                    log::warn!("branches: {err}");
                    for picker in &pickers {
                        picker.update(cx, |picker, cx| picker.set_loading(false, cx));
                    }
                }
            });
        });
    }

    fn install_branch_choices(
        &mut self,
        branches: Vec<Branch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ReviewSource::Branches { target, source } = &self.source else {
            return;
        };
        let index = |name: &str| branches.iter().position(|b| b.name == name);
        let checked = [index(target), index(source)];
        let entries = branch_entries(&branches);
        for (picker, checked) in [&self.branch_pickers.target, &self.branch_pickers.source]
            .into_iter()
            .zip(checked)
        {
            let entries = entries.clone();
            picker.update(cx, |picker, cx| {
                picker.set_entries(entries, checked, window, cx)
            });
        }
        self.branch_pickers.names = branches.into_iter().map(|b| b.name).collect();
    }

    fn pick_branch(&mut self, side: PullSide, entry: usize, cx: &mut Context<Self>) {
        let ReviewSource::Branches { target, source } = &self.source else {
            return;
        };
        let Some(name) = self.branch_pickers.names.get(entry) else {
            return;
        };
        let (new_target, new_source) = match side {
            PullSide::Target => (name, source),
            PullSide::Source => (target, name),
        };
        if new_target == new_source || (new_target == target && new_source == source) {
            return;
        }
        cx.emit(ReviewEvent::Open(ReviewSource::Branches {
            target: new_target.clone(),
            source: new_source.clone(),
        }));
    }
}
