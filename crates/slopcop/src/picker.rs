use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use git_repo::Branch;
use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter, Focusable as _,
    HighlightStyle, IntoElement, ParentElement as _, Pixels, ScrollStrategy, SharedString,
    Styled as _, StyledText, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IndexPath, Selectable, Sizable as _, h_flex,
    list::{List, ListDelegate, ListEvent, ListItem, ListState},
    popover::Popover,
    v_flex,
};
use review_core::{highlight_ranges, rank};

use crate::localization::tr;

const LIST_MAX_HEIGHT: Pixels = px(340.);

#[derive(Clone)]
pub struct PickerEntry {
    pub label: SharedString,
    pub detail: SharedString,
    pub search_detail: bool,
}

pub fn branch_entries(branches: &[Branch]) -> Vec<PickerEntry> {
    branches
        .iter()
        .map(|branch| {
            let sha = &branch.sha[..branch.sha.len().min(8)];
            PickerEntry {
                label: branch.name.clone().into(),
                detail: format!("{sha} · {}", branch.subject).into(),
                search_detail: false,
            }
        })
        .collect()
}

pub enum PickerEvent {
    Open,
    Confirm { entry: usize, secondary: bool },
}

type Footer = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub struct Picker {
    this: WeakEntity<Self>,
    list: Entity<ListState<PickerDelegate>>,
    placeholder: fn() -> SharedString,
    width: Pixels,
    footer: Option<Footer>,
    open: bool,
    _subscription: Subscription,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Picker {
    pub fn new(
        icon: impl Into<Icon>,
        placeholder: fn() -> SharedString,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = PickerDelegate {
            icon: icon.into(),
            entries: Arc::default(),
            rows: Vec::new(),
            query: String::new(),
            checked: None,
            selected: None,
            confirmed: None,
            loading: false,
            search: Task::ready(()),
        };
        let list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));
        let subscription = cx.subscribe_in(&list, window, Self::on_list_event);
        Self {
            this: cx.entity().downgrade(),
            list,
            placeholder,
            width,
            footer: None,
            open: false,
            _subscription: subscription,
        }
    }

    pub fn with_footer(
        mut self,
        footer: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.footer = Some(Rc::new(footer));
        self
    }

    pub fn set_entries(
        &mut self,
        entries: Vec<PickerEntry>,
        checked: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            delegate.entries = Arc::new(entries);
            delegate.checked = checked;
            delegate.loading = false;
            let query = delegate.query.clone();
            delegate.start_search(query, window, cx);
            cx.notify();
        });
    }

    pub fn set_checked(&mut self, checked: Option<usize>, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().checked = checked;
            cx.notify();
        });
    }

    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().loading = loading;
            cx.notify();
        });
    }

    pub fn is_empty(&self, cx: &App) -> bool {
        self.list.read(cx).delegate().entries.is_empty()
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            self.list
                .update(cx, |list, cx| list.set_query("", window, cx));
            cx.emit(PickerEvent::Open);
        }
        cx.notify();
    }

    fn on_list_event(
        &mut self,
        list: &Entity<ListState<PickerDelegate>>,
        event: &ListEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ListEvent::Confirm(_) => {
                let confirmed = list.update(cx, |list, _| list.delegate_mut().confirmed.take());
                self.close(cx);
                if let Some((entry, secondary)) = confirmed {
                    cx.emit(PickerEvent::Confirm { entry, secondary });
                }
            }
            ListEvent::Cancel => self.close(cx),
            ListEvent::Select(_) => {}
        }
    }

    pub fn popover<T>(&self, id: impl Into<ElementId>, trigger: T, cx: &App) -> Popover
    where
        T: Selectable + IntoElement + 'static,
    {
        let list = self.list.clone();
        let focus = list.focus_handle(cx);
        let this = self.this.clone();
        let footer = self.footer.clone();
        let placeholder = self.placeholder;
        let width = self.width;
        Popover::new(id)
            .p_0()
            .trigger(trigger)
            .open(self.open)
            .track_focus(&focus)
            .on_open_change(move |open, window, cx| {
                let _ = this.update(cx, |picker, cx| picker.set_open(*open, window, cx));
            })
            .content(move |_, window, cx| {
                let border = cx.theme().border;
                v_flex()
                    .w(width)
                    .child(
                        List::new(&list)
                            .small()
                            .search_placeholder(placeholder())
                            .max_h(LIST_MAX_HEIGHT)
                            .p_1(),
                    )
                    .when_some(footer.clone(), |el, footer| {
                        el.child(
                            div()
                                .p_1()
                                .border_t_1()
                                .border_color(border)
                                .child(footer(window, cx)),
                        )
                    })
            })
    }
}

struct PickerRow {
    entry: usize,
    label: Vec<Range<usize>>,
    detail: Vec<Range<usize>>,
}

pub struct PickerDelegate {
    icon: Icon,
    entries: Arc<Vec<PickerEntry>>,
    rows: Vec<PickerRow>,
    query: String,
    checked: Option<usize>,
    selected: Option<usize>,
    confirmed: Option<(usize, bool)>,
    loading: bool,
    search: Task<()>,
}

impl PickerDelegate {
    fn initial_row(&self) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }
        let checked_row = self
            .checked
            .filter(|_| self.query.trim().is_empty())
            .and_then(|checked| self.rows.iter().position(|row| row.entry == checked));
        Some(checked_row.unwrap_or(0))
    }

    fn start_search(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.query = query.clone();
        let entries = self.entries.clone();
        self.search = cx.spawn_in(window, async move |list, cx| {
            let rows = cx
                .background_spawn(async move { rank_rows(&query, &entries) })
                .await;
            let _ = list.update_in(cx, |list, window, cx| {
                let delegate = list.delegate_mut();
                delegate.rows = rows;
                let row = delegate.initial_row().map(IndexPath::new);
                list.set_selected_index(row, window, cx);
                if let Some(row) = row {
                    list.scroll_to_item(row, ScrollStrategy::Center, window, cx);
                }
                cx.notify();
            });
        });
    }
}

fn rank_rows(query: &str, entries: &[PickerEntry]) -> Vec<PickerRow> {
    let items = entries.iter().map(|entry| {
        (
            entry.label.as_ref(),
            entry.search_detail.then_some(entry.detail.as_ref()),
        )
    });
    rank(query, items)
        .into_iter()
        .map(|ranked| {
            let entry = &entries[ranked.index];
            PickerRow {
                entry: ranked.index,
                label: highlight_ranges(&entry.label, &ranked.label),
                detail: highlight_ranges(&entry.detail, &ranked.detail),
            }
        })
        .collect()
}

impl ListDelegate for PickerDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.start_search(query.to_string(), window, cx);
        Task::ready(())
    }

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.rows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.rows.get(ix.row)?;
        let entry = self.entries.get(row.entry)?;
        let theme = cx.theme();
        let highlight = HighlightStyle {
            color: Some(theme.blue),
            ..Default::default()
        };
        let label = StyledText::new(entry.label.clone())
            .with_highlights(row.label.iter().map(|range| (range.clone(), highlight)));
        let detail = StyledText::new(entry.detail.clone())
            .with_highlights(row.detail.iter().map(|range| (range.clone(), highlight)));
        Some(
            ListItem::new(("picker-row", ix.row))
                .px_2()
                .check_icon(IconName::Check)
                .confirmed(self.checked == Some(row.entry))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(
                            self.icon
                                .clone()
                                .small()
                                .flex_none()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_sm()
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        ),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .w_full()
            .py_4()
            .flex()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(tr().no_matches.clone())
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix.map(|ix| ix.row);
    }

    fn confirm(&mut self, secondary: bool, _: &mut Window, _: &mut Context<ListState<Self>>) {
        self.confirmed = self
            .selected
            .and_then(|row| self.rows.get(row))
            .map(|row| (row.entry, secondary));
    }
}
