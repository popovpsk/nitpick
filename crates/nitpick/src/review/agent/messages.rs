use gpui::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Subscription, div,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, h_flex,
    spinner::Spinner,
    text::{TextView, TextViewState},
    v_flex,
};

use super::links::{LinkKind, classify};
use super::{Activity, AgentPanel};
use crate::localization::tr;
use crate::review::OpenDiffLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    User,
    Assistant,
    Notice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Overview,
    Chat,
}

pub(super) struct Message {
    pub id: u64,
    pub role: Role,
    pub kind: Kind,
    pub head: String,
    pub text: String,
    notice: Option<SharedString>,
    view: Option<Entity<TextViewState>>,
    _observe: Option<Subscription>,
}

impl Message {
    pub(super) fn new(id: u64, role: Role, kind: Kind, head: String, text: String) -> Self {
        let notice = (role == Role::Notice).then(|| SharedString::from(text.clone()));
        Self {
            id,
            role,
            kind,
            head,
            text,
            notice,
            view: None,
            _observe: None,
        }
    }

    pub(super) fn append(&mut self, delta: &str, cx: &mut gpui::App) {
        self.text.push_str(delta);
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.push_str(delta, cx));
        }
    }
}

pub(super) fn agent_markdown(view: &Entity<TextViewState>) -> TextView {
    TextView::new(view)
        .selectable(true)
        .stream_fade(true)
        .link(|link, _| (classify(&link.url) != LinkKind::Ignored).then(|| link.clone()))
        .on_link_click(|url, event, window, cx| match classify(url) {
            LinkKind::Diff if event.standard_click() => {
                let url = url.to_string();
                window.dispatch_action(Box::new(OpenDiffLink { url }), cx);
            }
            LinkKind::Browser if event.standard_click() || event.is_middle_click() => {
                cx.open_url(url);
            }
            _ => {}
        })
}

impl AgentPanel {
    fn ensure_view(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.messages[ix].view.is_some() {
            return;
        }
        let text = self.messages[ix].text.clone();
        let id = self.messages[ix].id;
        let view = cx.new(|cx| TextViewState::markdown(&text, cx));
        let observe = cx.observe(&view, move |this, _, cx| this.row_changed(id, cx));
        let message = &mut self.messages[ix];
        message.view = Some(view);
        message._observe = Some(observe);
    }

    pub(super) fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        if ix >= self.messages.len() {
            return self.render_progress(cx);
        }
        self.ensure_view(ix, cx);
        let theme = cx.theme();
        let message = &self.messages[ix];
        let id = ElementId::NamedInteger("agent-message".into(), message.id);
        match message.role {
            Role::Notice => div()
                .id(id)
                .w_full()
                .px_2()
                .text_xs()
                .text_center()
                .text_color(theme.muted_foreground)
                .children(message.notice.clone())
                .into_any_element(),
            Role::User => {
                let text = message
                    .view
                    .as_ref()
                    .map(|view| agent_markdown(view).into_any_element());
                v_flex()
                    .id(id)
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(tr().assistant_you.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .rounded_lg()
                            .px_3()
                            .py_2()
                            .text_sm()
                            .bg(theme.secondary)
                            .text_color(theme.secondary_foreground)
                            .children(text),
                    )
                    .into_any_element()
            }
            Role::Assistant => {
                let text = message
                    .view
                    .as_ref()
                    .map(|view| agent_markdown(view).into_any_element());
                div()
                    .id(id)
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .children(text)
                    .into_any_element()
            }
        }
    }

    fn progress_label(&self) -> SharedString {
        let s = tr();
        match self.turn.as_ref().map(|turn| &turn.activity) {
            None => s.assistant_starting.clone(),
            Some(Activity::Working) => s.assistant_working.clone(),
            Some(Activity::Thinking) => s.assistant_thinking.clone(),
            Some(Activity::Tool(title) | Activity::Notice(title)) => title.clone(),
        }
    }

    fn render_progress(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id("agent-progress")
            .w_full()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Spinner::new().small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.progress_label()),
            )
            .into_any_element()
    }
}
