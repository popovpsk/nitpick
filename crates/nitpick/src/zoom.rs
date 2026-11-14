use gpui::{App, Global, actions};

use crate::settings_store::{
    CODE_ZOOM_RANGE, SettingsStore, apply_saved_theme, settings, update_settings,
};

actions!(zoom, [ZoomIn, ZoomOut, ZoomReset]);

#[derive(Default)]
struct AppliedZoom(u16, i16);

impl Global for AppliedZoom {}

pub fn init(cx: &mut App) {
    cx.on_action(|_: &ZoomIn, cx| step(1, cx));
    cx.on_action(|_: &ZoomOut, cx| step(-1, cx));
    cx.on_action(|_: &ZoomReset, cx| update_settings(cx, |s| s.code_zoom = 0));
    let current = settings(cx);
    cx.set_global(AppliedZoom(current.ui_scale, current.code_zoom));
    cx.observe_global::<SettingsStore>(|cx| {
        let current = settings(cx);
        let wanted = (current.ui_scale, current.code_zoom);
        let applied = cx.global::<AppliedZoom>();
        if (applied.0, applied.1) != wanted {
            cx.set_global(AppliedZoom(wanted.0, wanted.1));
            apply_saved_theme(cx);
        }
    })
    .detach();
}

fn step(delta: i16, cx: &mut App) {
    let (min, max) = CODE_ZOOM_RANGE;
    update_settings(cx, move |s| {
        s.code_zoom = (s.code_zoom + delta).clamp(min, max)
    });
}
