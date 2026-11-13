#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui::*;
use gpui_component::*;

use crate::main_view::MainView;

pub mod app_menu;
pub mod main_view;

pub(crate) fn build_window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.0), px(400.0))),
        titlebar: Some(TitlebarOptions {
            title: Some("Sloptaster".into()),
            ..TitleBar::title_bar_options()
        }),
        app_owns_titlebar_drag: true,
        app_id: Some("sloptaster".into()),
        #[cfg(target_os = "linux")]
        window_background: WindowBackgroundAppearance::Transparent,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

fn open_main_window(cx: &mut App) {
    let options = build_window_options(cx);
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| MainView::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("Failed to open window");
}

fn main() {
    let platform = gpui_platform::current_platform(false);
    let app = Application::with_platform(platform).with_assets(ui_resources::assets::Assets);

    #[cfg(target_os = "macos")]
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
    });

    app.run(move |cx| {
        let log_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("sloptaster")
            .join("logs");
        let _notices = diagnostics::init(diagnostics::Config {
            log_dir,
            ..Default::default()
        });

        gpui_component::init(cx);
        ui_resources::i18n::set_active(ui_resources::i18n::Lang::system());

        updater::init(cx, env!("CARGO_PKG_VERSION"), true);

        #[cfg(not(target_os = "macos"))]
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.on_app_quit(|_| {
            diagnostics::flush();
            async {}
        })
        .detach();

        cx.on_action(|_: &crate::app_menu::Quit, cx| cx.quit());

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
        cx.on_action(|_: &crate::app_menu::OpenRepository, cx| {
            cx.open_url(crate::app_menu::REPOSITORY_URL);
        });
        cx.on_action(|_: &updater::CheckForUpdates, cx| updater::check_now(cx));

        cx.activate(true);
        crate::app_menu::set_menus(cx);

        open_main_window(cx);
    });
}
