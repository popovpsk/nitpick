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
    pub menu_file: SharedString,
    pub menu_window: SharedString,
    pub minimize: SharedString,
    pub zoom: SharedString,
    pub menu_help: SharedString,
    pub repository: SharedString,
    pub up_to_date: SharedString,
    pub settings: SharedString,
    pub settings_ellipsis: SharedString,
    pub open: SharedString,
    pub open_repository: SharedString,
    pub open_repository_ellipsis: SharedString,
    pub open_repo_failed: SharedString,
    pub new_review: SharedString,
    pub new_window: SharedString,
    pub close_window: SharedString,
    pub welcome_hint: SharedString,
    pub choose_review: SharedString,
    pub choose_review_tooltip: SharedString,
    pub compare_branches: SharedString,
    pub target_branch: SharedString,
    pub source_branch: SharedString,
    pub choose_branch: SharedString,
    pub swap: SharedString,
    pub compare: SharedString,
    pub merge_requests: SharedString,
    pub mr_input_placeholder: SharedString,
    pub filter_mrs: SharedString,
    pub no_mrs: SharedString,
    pub gitlab_not_configured: SharedString,
    pub no_gitlab_remote: SharedString,
    pub no_remote_for_mr: SharedString,
    pub mr_not_ready: SharedString,
    pub loading_review: SharedString,
    pub retry: SharedString,
    pub files: SharedString,
    pub toggle_tree: SharedString,
    pub expand_all: SharedString,
    pub collapse_all: SharedString,
    pub mark_viewed: SharedString,
    pub mark_unviewed: SharedString,
    pub previous_file: SharedString,
    pub next_file: SharedString,
    pub split_view: SharedString,
    pub unified_view: SharedString,
    pub no_changes: SharedString,
    pub context_limited: SharedString,
    pub refresh: SharedString,
    pub open_in_browser: SharedString,
    pub general_comments: SharedString,
    pub comments_failed: SharedString,
    pub comment_failed: SharedString,
    pub comment_line_unavailable: SharedString,
    pub comment_placeholder: SharedString,
    pub comment_hint: SharedString,
    pub comment: SharedString,
    pub cancel: SharedString,
    pub reply: SharedString,
    pub resolve: SharedString,
    pub unresolve: SharedString,
    pub resolved: SharedString,
    pub outdated: SharedString,
    pub new_comment: SharedString,
    pub no_comments: SharedString,
    pub back: SharedString,
    pub appearance: SharedString,
    pub theme: SharedString,
    pub theme_desc: SharedString,
    pub system: SharedString,
    pub language: SharedString,
    pub review: SharedString,
    pub tree_side: SharedString,
    pub left: SharedString,
    pub right: SharedString,
    pub diff_layout: SharedString,
    pub ignore_whitespace: SharedString,
    pub ignore_whitespace_desc: SharedString,
    pub advance_on_viewed: SharedString,
    pub advance_on_viewed_desc: SharedString,
    pub auto_mark_viewed: SharedString,
    pub auto_mark_viewed_desc: SharedString,
    pub auto_mark_on: SharedString,
    pub auto_mark_off: SharedString,
    pub uncommitted_changes: SharedString,
    pub uncommitted_changes_desc: SharedString,
    pub gitlab_desc: SharedString,
    pub gitlab_url: SharedString,
    pub gitlab_token: SharedString,
    pub gitlab_token_desc: SharedString,
    pub save_and_check: SharedString,
    pub checking: SharedString,
    pub syntax_colors: SharedString,
    pub syntax_colors_dark_desc: SharedString,
    pub syntax_colors_light_desc: SharedString,
    pub same_as_theme: SharedString,
    pub zed_themes: SharedString,
    pub import_zed_theme: SharedString,
    pub import_zed_theme_desc: SharedString,
    pub installed_in_zed: SharedString,
    pub choose_file: SharedString,
    pub zed_theme_prompt: SharedString,

    pub add_repositories_ellipsis: SharedString,
    pub switch_repository_tooltip: SharedString,
    pub search_repositories: SharedString,
    pub search_branches: SharedString,
    pub no_matches: SharedString,
    pub no_new_repositories: SharedString,
    pub scan_failed: SharedString,
    pub change_target_branch: SharedString,
    pub change_source_branch: SharedString,
    pub compare_tooltip: SharedString,

    pub update_ready_t: SharedString,
    pub update_check_failed_t: SharedString,
    pub settings_save_failed_t: SharedString,
    pub gitlab_connected_t: SharedString,
    pub merge_base_t: SharedString,
    pub pull_branch_t: SharedString,
    pub pull_failed_t: SharedString,
    pub pull_up_to_date_t: SharedString,
    pub pull_updated_t: SharedString,

    pub repositories_added_t: SharedString,
    pub viewed_progress_t: SharedString,
    pub zed_imported_t: SharedString,
    pub zed_import_failed_t: SharedString,
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

    pub fn settings_save_failed(&self, err: &str) -> String {
        fill(&self.settings_save_failed_t, &[err])
    }

    pub fn gitlab_connected(&self, user: &str) -> String {
        fill(&self.gitlab_connected_t, &[user])
    }

    pub fn merge_base(&self, base: &str, commits: usize) -> String {
        fill(&self.merge_base_t, &[base, &commits.to_string()])
    }

    pub fn pull_branch(&self, branch: &str) -> String {
        fill(&self.pull_branch_t, &[branch])
    }

    pub fn pull_failed(&self, branch: &str) -> String {
        fill(&self.pull_failed_t, &[branch])
    }

    pub fn pull_up_to_date(&self, branch: &str) -> String {
        fill(&self.pull_up_to_date_t, &[branch])
    }

    pub fn pull_updated(&self, branch: &str) -> String {
        fill(&self.pull_updated_t, &[branch])
    }

    pub fn repositories_added(&self, count: usize) -> String {
        fill(&self.repositories_added_t, &[&count.to_string()])
    }

    pub fn viewed_progress(&self, viewed: usize, total: usize) -> String {
        fill(
            &self.viewed_progress_t,
            &[&viewed.to_string(), &total.to_string()],
        )
    }

    pub fn zed_imported(&self, themes: &str) -> String {
        fill(&self.zed_imported_t, &[themes])
    }

    pub fn zed_import_failed(&self, err: &str) -> String {
        fill(&self.zed_import_failed_t, &[err])
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
            for template in [
                &s.update_ready_t,
                &s.update_check_failed_t,
                &s.settings_save_failed_t,
                &s.gitlab_connected_t,
                &s.pull_branch_t,
                &s.pull_failed_t,
                &s.pull_up_to_date_t,
                &s.pull_updated_t,
                &s.repositories_added_t,
                &s.zed_imported_t,
                &s.zed_import_failed_t,
            ] {
                assert_eq!(template.matches("{}").count(), 1, "{lang:?}: {template}");
            }
            assert_eq!(s.merge_base_t.matches("{}").count(), 2, "{lang:?}");
            assert_eq!(s.viewed_progress_t.matches("{}").count(), 2, "{lang:?}");
        }
    }
}
