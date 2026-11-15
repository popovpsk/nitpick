use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, SystemWindowTabController, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

use super::NewWindow;

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
    let theme = cx.theme();
    let mut row = h_flex()
        .h(px(30.))
        .flex_none()
        .w_full()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border);
    for (ix, tab) in tabs.iter().enumerate() {
        let active = tab.id == current;
        let handle = tab.handle;
        let close_handle = tab.handle;
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
                .on_click(move |_, _, cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }),
        );
    }
    row.child(
        div().px_1().child(
            Button::new("window-tab-new")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
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
