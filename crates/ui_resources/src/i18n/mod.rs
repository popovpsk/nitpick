use std::sync::atomic::{AtomicU8, Ordering};

use gpui::SharedString;

mod en;
mod ru;

pub struct Strings {
    pub check_for_updates: SharedString,
    pub hide_app: SharedString,
    pub hide_others: SharedString,
    pub show_all: SharedString,
    pub quit_app: SharedString,
    pub menu_window: SharedString,
    pub minimize: SharedString,
    pub zoom: SharedString,
    pub menu_help: SharedString,
    pub repository: SharedString,

    pub up_to_date: SharedString,
    pub update_ready_t: SharedString,
    pub update_check_failed_t: SharedString,
}

fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut parts = template.split("{}");
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for (arg, part) in args.iter().zip(parts) {
        out.push_str(arg);
        out.push_str(part);
    }
    out
}

impl Strings {
    pub fn update_ready(&self, version: &str) -> String {
        fill(&self.update_ready_t, &[version])
    }

    pub fn update_check_failed(&self, err: &str) -> String {
        fill(&self.update_check_failed_t, &[err])
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! lang {
    ($($key:ident : $val:literal),* $(,)?) => {
        $crate::i18n::Strings {
            $( $key: ::gpui::SharedString::new_static($val) ),*
        }
    };
}

macro_rules! languages {
    ($($variant:ident => ($code:literal, $name:literal, $table:path)),* $(,)?) => {
        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        pub enum Lang {
            $($variant),*
        }

        impl Lang {
            pub fn all() -> &'static [Lang] {
                &[$(Lang::$variant),*]
            }

            pub fn code(self) -> &'static str {
                match self { $(Lang::$variant => $code),* }
            }

            pub fn display_name(self) -> &'static str {
                match self { $(Lang::$variant => $name),* }
            }

            pub fn strings(self) -> &'static Strings {
                match self { $(Lang::$variant => &$table),* }
            }

            pub fn from_code(code: &str) -> Option<Lang> {
                match code {
                    $($code => Some(Lang::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

languages! {
    En => ("en", "English", en::EN),
    Ru => ("ru", "Русский", ru::RU),
}

impl Lang {
    pub fn from_locale(locale: &str) -> Lang {
        let prefix = locale
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        Self::from_code(&prefix).unwrap_or(Lang::En)
    }

    pub fn system() -> Lang {
        sys_locale::get_locale()
            .map(|locale| Self::from_locale(&locale))
            .unwrap_or(Lang::En)
    }

    fn index(self) -> u8 {
        Self::all().iter().position(|&l| l == self).unwrap_or(0) as u8
    }

    fn from_index(i: u8) -> Lang {
        Self::all().get(i as usize).copied().unwrap_or(Lang::En)
    }
}

static ACTIVE_LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_active(lang: Lang) {
    ACTIVE_LANG.store(lang.index(), Ordering::Relaxed);
}

pub fn active() -> Lang {
    Lang::from_index(ACTIVE_LANG.load(Ordering::Relaxed))
}

pub fn strings() -> &'static Strings {
    active().strings()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_handles_placeholders() {
        assert_eq!(fill("v{} ready", &["1.2.3"]), "v1.2.3 ready");
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn locale_maps_to_language() {
        assert_eq!(Lang::from_locale("ru-RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("ru_RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("xx"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }

    #[test]
    fn every_language_round_trips() {
        for &lang in Lang::all() {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
            assert_eq!(Lang::from_index(lang.index()), lang);
        }
    }

    #[test]
    fn interpolated_templates_have_one_placeholder() {
        for &lang in Lang::all() {
            let s = lang.strings();
            for template in [&s.update_ready_t, &s.update_check_failed_t] {
                assert_eq!(template.matches("{}").count(), 1, "{lang:?}: {template}");
            }
        }
    }
}
