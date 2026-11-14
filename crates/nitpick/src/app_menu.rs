use gpui::{App, Menu, MenuItem, actions};

use crate::localization::tr;
use crate::workspace::{
    AddRepositories, CloseWindow, NewReview, NewWindow, OpenRepository, ShowSettings,
};

actions!(
    nitpick,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        OpenProjectPage,
    ]
);

pub(crate) const REPOSITORY_URL: &str = "https://github.com/popovpsk/nitpick/";

pub fn app_menus() -> Vec<Menu> {
    let s = tr();
    let mut app_items = Vec::new();
    if updater::is_supported() {
        app_items.push(MenuItem::action(
            s.check_for_updates.clone(),
            updater::CheckForUpdates,
        ));
        app_items.push(MenuItem::separator());
    }
    app_items.push(MenuItem::action(s.settings_ellipsis.clone(), ShowSettings));
    app_items.push(MenuItem::separator());
    #[cfg(target_os = "macos")]
    {
        app_items.push(MenuItem::os_submenu(
            "Services",
            gpui::SystemMenuType::Services,
        ));
        app_items.push(MenuItem::separator());
        app_items.push(MenuItem::action(s.hide_app.clone(), Hide));
        app_items.push(MenuItem::action(s.hide_others.clone(), HideOthers));
        app_items.push(MenuItem::action(s.show_all.clone(), ShowAll));
        app_items.push(MenuItem::separator());
    }
    app_items.push(MenuItem::action(s.quit_app.clone(), Quit));
    vec![
        Menu {
            name: "Nitpick".into(),
            items: app_items,
            disabled: false,
        },
        Menu {
            name: s.menu_file.clone(),
            items: vec![
                MenuItem::action(s.open_repository_ellipsis.clone(), OpenRepository),
                MenuItem::action(s.add_repositories_ellipsis.clone(), AddRepositories),
                MenuItem::action(s.new_review.clone(), NewReview),
                MenuItem::separator(),
                MenuItem::action(s.new_window.clone(), NewWindow),
                MenuItem::action(s.close_window.clone(), CloseWindow),
            ],
            disabled: false,
        },
        #[cfg(target_os = "macos")]
        Menu {
            name: s.menu_window.clone(),
            items: vec![
                MenuItem::action(s.minimize.clone(), Minimize),
                MenuItem::action(s.zoom.clone(), Zoom),
            ],
            disabled: false,
        },
        Menu {
            name: s.menu_help.clone(),
            items: vec![MenuItem::action(s.repository.clone(), OpenProjectPage)],
            disabled: false,
        },
    ]
}

pub fn set_menus(cx: &App) {
    cx.set_menus(app_menus());
}
