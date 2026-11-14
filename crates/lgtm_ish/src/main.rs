#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use gpui::*;
use gpui_component::*;

use crate::workspace::Workspace;

pub mod app_menu;
pub mod keymap;
pub mod localization;
pub mod picker;
pub mod recent;
pub mod review;
pub mod settings_store;
pub mod settings_view;
pub mod shortcut_dialog;
pub mod window_registry;
pub mod workspace;

const TABBING_IDENTIFIER: &str = "lgtm-ish";

pub(crate) fn build_window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.0), px(480.0))),
        titlebar: Some(TitlebarOptions {
            title: Some("LGTM-ish".into()),
            ..TitleBar::title_bar_options()
        }),
        app_owns_titlebar_drag: true,
        app_id: Some("lgtm-ish".into()),
        tabbing_identifier: Some(TABBING_IDENTIFIER.into()),
        #[cfg(target_os = "linux")]
        window_background: WindowBackgroundAppearance::Transparent,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

pub fn open_window(repo: Option<PathBuf>, cx: &mut App) {
    let options = build_window_options(cx);
    let result = cx.open_window(options, |window, cx| {
        window_registry::register(window, cx);
        let view = cx.new(|cx| Workspace::new(repo, window, cx));
        if window.focused(cx).is_none() {
            let focus = view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(err) = result {
        log::error!("opening a window: {err}");
    }
}

fn open_startup_windows(cx: &mut App) {
    let repos: Vec<PathBuf> = recent::recent(cx)
        .open_windows()
        .iter()
        .filter(|p| p.is_dir())
        .cloned()
        .collect();
    if repos.is_empty() {
        open_window(None, cx);
    } else {
        for repo in repos {
            open_window(Some(repo), cx);
        }
    }
}

fn main() {
    let platform = gpui_platform::current_platform(false);
    let app = Application::with_platform(platform).with_assets(ui_resources::assets::Assets);

    #[cfg(target_os = "macos")]
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_window(None, cx);
        }
    });

    app.run(move |cx| {
        let data_dir = settings_store::app_dir(dirs::data_dir());
        let _notices = diagnostics::init(diagnostics::Config {
            log_dir: data_dir.join("logs"),
            ..Default::default()
        });

        gpui_component::init(cx);
        cx.set_global(settings_store::SettingsStore::load());
        settings_store::observe_theme_for_syntax_colors(cx);
        cx.set_global(recent::RecentRepos::load(data_dir.join("recent.json")));
        localization::sync_active_lang(cx);
        settings_store::apply_saved_theme(cx);
        ui_resources::themes::register_bundled_themes(cx, settings_store::apply_saved_theme);

        settings_view::init(cx);
        updater::init(cx, env!("CARGO_PKG_VERSION"), true);
        keymap::init(cx);

        cx.on_window_closed(|cx, id| {
            window_registry::remove(id, cx);
            #[cfg(not(target_os = "macos"))]
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.on_app_quit(|cx| {
            window_registry::quitting(cx);
            review::flush_viewed(cx);
            diagnostics::flush();
            async {}
        })
        .detach();

        cx.on_action(|_: &crate::app_menu::Quit, cx| {
            window_registry::quitting(cx);
            cx.quit();
        });

        #[cfg(target_os = "macos")]
        cx.on_action(|_: &crate::app_menu::Hide, cx| cx.hide());
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &crate::app_menu::HideOthers, cx| cx.hide_other_apps());
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &crate::app_menu::ShowAll, cx| cx.unhide_other_apps());
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &crate::app_menu::Minimize, cx| {
            if let Some(w) = cx.active_window() {
                let _ = w.update(cx, |_, window, _| window.minimize_window());
            }
        });
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &crate::app_menu::Zoom, cx| {
            if let Some(w) = cx.active_window() {
                let _ = w.update(cx, |_, window, _| window.zoom_window());
            }
        });
        cx.on_action(|_: &crate::app_menu::OpenProjectPage, cx| {
            cx.open_url(crate::app_menu::REPOSITORY_URL);
        });
        cx.on_action(|_: &workspace::NewWindow, cx| open_window(None, cx));
        cx.on_action(|_: &workspace::OpenRepository, cx| open_window(None, cx));
        cx.on_action(|_: &workspace::AddRepositories, cx| open_window(None, cx));
        cx.on_action(|_: &updater::CheckForUpdates, cx| updater::check_now(cx));

        cx.activate(true);
        crate::app_menu::set_menus(cx);

        open_startup_windows(cx);
    });
}
