use gpui::App;
use ui_resources::i18n::{Lang, Strings};

use crate::settings_store::{LangChoice, settings};

pub fn resolve(cx: &App) -> Lang {
    match &settings(cx).language {
        LangChoice::System => Lang::system(),
        LangChoice::Named(code) => Lang::from_code(code).unwrap_or(Lang::En),
    }
}

pub fn sync_active_lang(cx: &App) {
    ui_resources::i18n::set_active(resolve(cx));
}

pub fn tr() -> &'static Strings {
    ui_resources::i18n::strings()
}
