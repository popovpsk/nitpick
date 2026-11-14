use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AppContext as _, Context,
    InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, pulsating_between, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    v_flex,
};
use ui_resources::Icons;
use ui_resources::i18n::Strings;

use crate::keymap::{self, Command, Group};
use crate::localization::tr;

const MODIFIER_KEYS: [&str; 9] = [
    "shift", "control", "ctrl", "alt", "platform", "cmd", "fn", "function", "capslock",
];

pub fn where_label(group: Group, s: &Strings) -> SharedString {
    match group {
        Group::Review => s.where_review.clone(),
        Group::Tree => s.where_tree.clone(),
        Group::Window => s.where_window.clone(),
    }
}

pub fn keystroke_element(key: &str, cx: &App) -> AnyElement {
    match Keystroke::parse_with_os(key, cx.platform_info().os) {
        Ok(keystroke) => Kbd::new(keystroke).into_any_element(),
        Err(_) => div().text_sm().child(key.to_string()).into_any_element(),
    }
}

pub fn open(
    command: &'static Command,
    original: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let recorder = cx.new(|cx| ShortcutRecorder::new(command, original, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let s = tr();
        let state = recorder.read(cx);
        let can_save = state.recorded.is_some();
        let can_remove = state.original.is_some();
        let confirm = recorder.clone();
        let save = recorder.clone();
        let remove = recorder.clone();
        dialog
            .title((command.title)(s))
            .w(px(460.))
            .on_ok(move |_, _, cx| confirm.update(cx, |this, cx| this.save(cx)))
            .child(recorder.clone())
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(can_remove, |el| {
                        el.child(
                            Button::new("shortcut-remove")
                                .ghost()
                                .small()
                                .icon(Icon::new(Icons::CircleX))
                                .label(s.remove.clone())
                                .on_click(move |_, window, cx| {
                                    remove.update(cx, |this, cx| this.remove(cx));
                                    window.close_dialog(cx);
                                }),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("shortcut-cancel")
                            .outline()
                            .small()
                            .label(s.cancel.clone())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("shortcut-save")
                            .primary()
                            .small()
                            .label(s.save.clone())
                            .disabled(!can_save)
                            .on_click(move |_, window, cx| {
                                if save.update(cx, |this, cx| this.save(cx)) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}

pub struct ShortcutRecorder {
    command: &'static Command,
    original: Option<String>,
    recorded: Option<String>,
    recording: bool,
    window: AnyWindowHandle,
    _intercept: Option<Subscription>,
}

impl ShortcutRecorder {
    fn new(
        command: &'static Command,
        original: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            command,
            recorded: original.clone(),
            original,
            recording: false,
            window: window.window_handle(),
            _intercept: None,
        };
        this.start(cx);
        this
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.recording = true;
        let handle = self.window;
        let this = cx.entity().downgrade();
        self._intercept = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let consumed = this
                .update(cx, |this, cx| this.on_keystroke(&event.keystroke, cx))
                .unwrap_or(false);
            if consumed {
                cx.stop_propagation();
            }
        }));
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.recording = false;
        self._intercept = None;
        cx.notify();
    }

    fn on_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if !self.recording {
            return false;
        }
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            self.stop(cx);
            return true;
        }
        if keystroke.key.is_empty() || MODIFIER_KEYS.contains(&keystroke.key.as_str()) {
            return true;
        }
        self.recorded = Some(keystroke.unparse(cx.platform_info().os));
        self.stop(cx);
        true
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.recorded.clone() else {
            return false;
        };
        self.stop(cx);
        if self.original.as_deref() != Some(key.as_str()) {
            keymap::assign(self.command, self.original.clone(), Some(key), cx);
        }
        true
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        keymap::assign(self.command, self.original.clone(), None, cx);
    }
}

impl Render for ShortcutRecorder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = tr();
        let theme = cx.theme();
        let recording = self.recording;
        let conflict = self
            .recorded
            .as_ref()
            .filter(|key| self.original.as_ref() != Some(*key))
            .and_then(|key| keymap::conflict(key, self.command, cx));
        let shown = match &self.recorded {
            Some(key) if !recording => keystroke_element(key, cx),
            _ => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(if recording {
                    s.shortcut_recording.clone()
                } else {
                    s.shortcut_none.clone()
                })
                .into_any_element(),
        };
        let badge = if recording {
            h_flex()
                .flex_none()
                .gap_1()
                .px_1p5()
                .py_0p5()
                .items_center()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.danger.opacity(0.4))
                .child(
                    div()
                        .size(px(7.))
                        .rounded_full()
                        .bg(theme.danger)
                        .with_animation(
                            "recording-pulse",
                            Animation::new(Duration::from_millis(1600))
                                .repeat()
                                .with_easing(pulsating_between(0.3, 1.0)),
                            |el, delta| el.opacity(delta),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.danger)
                        .child("REC"),
                )
                .into_any_element()
        } else {
            Button::new("shortcut-record")
                .ghost()
                .xsmall()
                .icon(Icon::new(Icons::CircleDot))
                .label(s.shortcut_record.clone())
                .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
                .into_any_element()
        };
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(where_label(self.command.group, s)),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_sm().child(s.shortcut_label.clone()))
                    .child(
                        h_flex()
                            .id("shortcut-box")
                            .h(px(44.))
                            .px_3()
                            .gap_2()
                            .items_center()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(if recording {
                                theme.danger.opacity(0.6)
                            } else {
                                theme.input
                            })
                            .bg(if recording {
                                theme.danger.opacity(0.06)
                            } else {
                                theme.background
                            })
                            .cursor_pointer()
                            .when(!recording, |el| {
                                el.on_click(cx.listener(|this, _, _, cx| this.start(cx)))
                            })
                            .child(div().flex_1().min_w_0().child(shown))
                            .child(badge),
                    ),
            )
            .when_some(conflict, |el, other| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(s.shortcut_conflict(&(other.title)(s))),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(s.shortcut_hint.clone()),
            )
    }
}
