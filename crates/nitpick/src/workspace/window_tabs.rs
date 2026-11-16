use gpui::{
    AnyElement, App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    SystemWindowTabController, Window, WindowId, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

use super::{CloseWindow, NewWindow};
use crate::localization::tr;

const STRIP_HEIGHT: f32 = 30.;
const NEW_TAB_REMS: f32 = 1.75;

#[derive(Clone)]
struct DraggedWindowTab {
    id: WindowId,
    ix: usize,
    title: SharedString,
    active: bool,
    width: Pixels,
}

impl Render for DraggedWindowTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h(px(STRIP_HEIGHT))
            .w(self.width)
            .px_7()
            .justify_center()
            .text_sm()
            .bg(if self.active {
                theme.tab_active
            } else {
                theme.tab_bar
            })
            .text_color(if self.active {
                theme.tab_active_foreground
            } else {
                theme.tab_foreground
            })
            .border_1()
            .border_color(theme.border)
            .font_family(theme.font_family.clone())
            .opacity(0.9)
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.title.clone()),
            )
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    let current = window.window_handle().window_id();
    let Some(tabs) = cx
        .global::<SystemWindowTabController>()
        .tabs(current)
        .filter(|tabs| tabs.len() > 1)
        .cloned()
    else {
        return div().into_any_element();
    };
    let tab_width =
        (window.viewport_size().width - window.rem_size() * NEW_TAB_REMS) / tabs.len() as f32;
    let theme = cx.theme();
    let mut row = h_flex()
        .h(px(STRIP_HEIGHT))
        .flex_none()
        .w_full()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border);
    for (ix, tab) in tabs.iter().enumerate() {
        let active = tab.id == current;
        let handle = tab.handle;
        let close_handle = tab.handle;
        let dragged = DraggedWindowTab {
            id: tab.id,
            ix,
            title: tab.title.clone(),
            active,
            width: tab_width,
        };
        row = row.child(
            h_flex()
                .id(("window-tab", ix))
                .group("window-tab")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .px_7()
                .items_center()
                .justify_center()
                .text_sm()
                .cursor_pointer()
                .border_r_1()
                .border_color(theme.border)
                .when(active, |el| {
                    el.bg(theme.tab_active)
                        .text_color(theme.tab_active_foreground)
                })
                .when(!active, |el| {
                    el.text_color(theme.tab_foreground)
                        .hover(|s| s.bg(theme.list_hover))
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(tab.title.clone()),
                )
                .child(
                    h_flex()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_1()
                        .items_center()
                        .invisible()
                        .group_hover("window-tab", |s| s.visible())
                        .child(
                            Button::new(("window-tab-close", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .when(active, |button| {
                                    button.tooltip_with_action(
                                        tr().close_window.clone(),
                                        &CloseWindow,
                                        Some("Workspace"),
                                    )
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    if active {
                                        window.remove_window();
                                    } else {
                                        let _ = close_handle
                                            .update(cx, |_, window, _| window.remove_window());
                                    }
                                }),
                        ),
                )
                .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                    if active {
                        window.remove_window();
                    } else {
                        let _ = close_handle.update(cx, |_, window, _| window.remove_window());
                    }
                })
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                .drag_over::<DraggedWindowTab>(move |style, dragged, _, cx| {
                    let style = style.bg(cx.theme().drop_target);
                    if ix < dragged.ix {
                        style
                            .border_0()
                            .border_l_2()
                            .border_color(cx.theme().drag_border)
                    } else if ix > dragged.ix {
                        style
                            .border_0()
                            .border_r_2()
                            .border_color(cx.theme().drag_border)
                    } else {
                        style
                    }
                })
                .on_drop(move |dragged: &DraggedWindowTab, _, cx| {
                    SystemWindowTabController::update_tab_position(cx, dragged.id, ix);
                })
                .on_click(move |_, _, cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }),
        );
    }
    row.child(
        div().w(rems(NEW_TAB_REMS)).flex_none().px_1().child(
            Button::new("window-tab-new")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip_with_action(tr().new_window.clone(), &NewWindow, Some("Workspace"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(NewWindow), cx)),
        ),
    )
    .into_any_element()
}

pub fn activate_next(window: &mut Window, cx: &mut App, delta: isize) {
    let current = window.window_handle().window_id();
    let Some(tabs) = cx
        .global::<SystemWindowTabController>()
        .tabs(current)
        .cloned()
    else {
        return;
    };
    let Some(pos) = tabs.iter().position(|t| t.id == current) else {
        return;
    };
    let next = (pos as isize + delta).rem_euclid(tabs.len() as isize) as usize;
    let handle = tabs[next].handle;
    let _ = handle.update(cx, |_, window, _| window.activate_window());
}
