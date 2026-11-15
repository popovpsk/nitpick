use std::path::PathBuf;

use gpui::*;
use gpui_component::{Root, TitleBar};

use crate::localization::tr;
use crate::settings_view::{SettingsSection, SettingsView};
use crate::workspace::{CloseWindow, ShowSettings};

pub struct Opener {
    pub window: WindowId,
    pub display: Option<DisplayId>,
    pub repo: Option<PathBuf>,
}

impl Opener {
    pub fn of(window: &Window, repo: Option<PathBuf>, cx: &App) -> Self {
        Self {
            window: window.window_handle().window_id(),
            display: window.display(cx).map(|display| display.id()),
            repo,
        }
    }
}

#[derive(Default)]
struct SettingsWindow {
    open: Option<(WindowHandle<Root>, WeakEntity<SettingsView>)>,
    owner: Option<WindowId>,
}

impl Global for SettingsWindow {}

pub fn init(cx: &mut App) {
    cx.on_action(|_: &ShowSettings, cx| {
        let opener = cx.active_window().and_then(|handle| {
            let id = handle.window_id();
            let repo = crate::window_registry::repo_of(id, cx)?;
            Some(Opener {
                window: id,
                display: None,
                repo,
            })
        });
        cx.defer(move |cx| open(opener, None, cx));
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(handle) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    });
}

pub fn open(opener: Option<Opener>, section: Option<SettingsSection>, cx: &mut App) {
    if let Some((handle, view)) = cx.default_global::<SettingsWindow>().open.clone()
        && let Some(view) = view.upgrade()
    {
        let repo = opener.as_ref().map(|o| o.repo.clone());
        let shown = handle.update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                if let Some(repo) = repo {
                    view.set_repository(repo, window, cx);
                }
                if let Some(section) = section {
                    view.reveal(section, cx);
                }
            });
            window.activate_window();
            let focus = view.focus_handle(cx);
            window.focus(&focus, cx);
        });
        if shown.is_ok() {
            if let Some(opener) = opener {
                cx.global_mut::<SettingsWindow>().owner = Some(opener.window);
            }
            return;
        }
    }
    let display = opener.as_ref().and_then(|o| o.display);
    let owner = opener.as_ref().map(|o| o.window);
    let repo = opener.and_then(|o| o.repo);
    let options = window_options(display, cx);
    let mut created = None;
    let result = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| {
            let mut view = SettingsView::new(window, cx);
            view.set_repository(repo, window, cx);
            if let Some(section) = section {
                view.reveal(section, cx);
            }
            view
        });
        let focus = view.focus_handle(cx);
        window.focus(&focus, cx);
        created = Some(view.downgrade());
        cx.new(|cx| Root::new(view, window, cx))
    });
    match (result, created) {
        (Ok(handle), Some(view)) => {
            let state = cx.global_mut::<SettingsWindow>();
            state.open = Some((handle, view));
            state.owner = owner;
        }
        (Err(err), _) => log::error!("opening the settings window: {err}"),
        _ => {}
    }
}

pub fn repository_changed(window: WindowId, repo: Option<PathBuf>, cx: &mut App) {
    let Some(state) = cx.try_global::<SettingsWindow>() else {
        return;
    };
    if state.owner != Some(window) {
        return;
    }
    let Some((handle, view)) = state.open.clone() else {
        return;
    };
    let Some(view) = view.upgrade() else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        view.update(cx, |view, cx| view.set_repository(repo, window, cx));
    });
}

fn window_options(display: Option<DisplayId>, cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(display, size(px(900.0), px(720.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.0), px(420.0))),
        titlebar: Some(TitlebarOptions {
            title: Some(tr().settings.clone()),
            ..TitleBar::title_bar_options()
        }),
        app_owns_titlebar_drag: true,
        app_id: Some("nitpick".into()),
        display_id: display,
        kind: WindowKind::Floating,
        tabbing_identifier: None,
        #[cfg(target_os = "linux")]
        window_background: WindowBackgroundAppearance::Transparent,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}
