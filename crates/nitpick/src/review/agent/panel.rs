use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, CursorStyle,
    DragMoveEvent, ElementId, Empty, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
    ease_out_quint, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ui_resources::Icons;

use super::{AgentEvent, AgentPanel, Lookup, Phase, SLIDE};
use crate::localization::tr;
use crate::review::diff_panel::TOOLBAR_HEIGHT_REMS;

#[derive(Clone)]
struct AgentResize;

impl Render for AgentResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(in crate::review) fn frame(panel: &Entity<AgentPanel>, cx: &App) -> Option<AnyElement> {
    let state = panel.read(cx);
    if !(state.open || state.closing) {
        return None;
    }
    let theme = cx.theme();
    let width = state.width;
    let closing = state.closing;
    let toggles = state.toggles;
    let resizing = panel.clone();
    let handle = div()
        .id("agent-resize")
        .absolute()
        .top_0()
        .left(px(-2.))
        .h_full()
        .w(px(5.))
        .cursor(CursorStyle::ResizeLeftRight)
        .hover(|el| el.bg(theme.drag_border))
        .on_drag(AgentResize, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        });
    let frame = div()
        .id("agent-panel")
        .relative()
        .flex_none()
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .on_drag_move(move |event: &DragMoveEvent<AgentResize>, _, cx| {
            let width = event.bounds.right() - event.event.position.x;
            resizing.update(cx, |panel, cx| panel.resize(f32::from(width), cx));
        })
        .child(
            div()
                .size_full()
                .overflow_hidden()
                .child(div().h_full().w(px(width)).child(panel.clone())),
        )
        .when(!closing, |el| el.child(handle))
        .when(closing, |el| {
            el.child(
                div()
                    .id("agent-closing")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude(),
            )
        });
    Some(
        frame
            .with_animation(
                ElementId::NamedInteger("agent-slide".into(), toggles as u64),
                Animation::new(SLIDE).with_easing(ease_out_quint()),
                move |el, delta| {
                    let factor = if closing { 1.0 - delta } else { delta };
                    el.w(px(width * factor))
                },
            )
            .into_any_element(),
    )
}

impl AgentPanel {
    fn render_title(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let theme = cx.theme();
        h_flex()
            .flex_none()
            .h(rems(TOOLBAR_HEIGHT_REMS))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(Icon::new(Icons::Sparkles).small().text_color(theme.primary))
            .child(div().flex_none().font_semibold().child(s.assistant.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(self.profile_name.clone()),
            )
            .child(
                Button::new("agent-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(s.assistant_close.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
            .into_any_element()
    }

    fn render_missing(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let theme = cx.theme();
        centered()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .large()
                    .text_color(theme.warning),
            )
            .child(
                div()
                    .font_semibold()
                    .child(s.assistant_missing_title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(self.missing_hint.clone()),
            )
            .child(
                Button::new("agent-open-settings")
                    .small()
                    .label(s.assistant_open_settings.clone())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentEvent::OpenSettings))),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = tr();
        let theme = cx.theme();
        centered()
            .child(
                Icon::new(Icons::Sparkles)
                    .large()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .font_semibold()
                    .child(s.assistant_overview_title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(s.assistant_overview_hint.clone()),
            )
            .when_some(self.failure(), |el, message| {
                el.child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(theme.danger)
                        .child(message),
                )
            })
            .child(
                Button::new("agent-request-overview")
                    .primary()
                    .small()
                    .label(s.assistant_request_overview.clone())
                    .disabled(!self.can_request_overview())
                    .on_click(cx.listener(|this, _, _, cx| this.request_overview(cx))),
            )
            .into_any_element()
    }

    fn failure(&self) -> Option<gpui::SharedString> {
        match &self.phase {
            Phase::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }
}

fn centered() -> gpui::Div {
    v_flex()
        .size_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
}

impl Render for AgentPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.sidebar;
        let title = self.render_title(cx);
        let body = match self.lookup {
            Lookup::Missing => self.render_missing(cx),
            Lookup::Checking | Lookup::Found => self.render_empty(cx),
        };
        v_flex()
            .id("agent-panel-content")
            .size_full()
            .min_w_0()
            .bg(background)
            .child(title)
            .child(div().flex_1().min_h_0().child(body))
    }
}
