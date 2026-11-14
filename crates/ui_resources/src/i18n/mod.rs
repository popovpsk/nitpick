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
    pub show_whole_file: SharedString,
    pub show_whole_file_desc: SharedString,
    pub wrap_lines: SharedString,
    pub wrap_split_only: SharedString,
    pub wrap_always: SharedString,
    pub wrap_never: SharedString,
    pub reset_progress: SharedString,
    pub reset_progress_ellipsis: SharedString,
    pub reset_progress_title: SharedString,
    pub reset_progress_desc_t: SharedString,
    pub find_file: SharedString,
    pub find_file_placeholder: SharedString,
    pub no_matching_files: SharedString,
    pub find_in_file: SharedString,
    pub find_in_file_placeholder: SharedString,
    pub find_next: SharedString,
    pub find_previous: SharedString,
    pub no_results: SharedString,
    pub advance_on_viewed: SharedString,
    pub advance_on_viewed_desc: SharedString,
    pub auto_mark_viewed: SharedString,
    pub tree_view_label: SharedString,
    pub cmd_zoom_in: SharedString,
    pub cmd_zoom_out: SharedString,
    pub cmd_zoom_reset: SharedString,
    pub interface_size: SharedString,
    pub code_text_size: SharedString,
    pub code_text_size_desc: SharedString,
    pub tree_mode_tree: SharedString,
    pub tree_mode_list: SharedString,
    pub compact_folders: SharedString,
    pub show_hidden_files: SharedString,
    pub tree_view_options: SharedString,
    pub shortcut_needs_modifier: SharedString,
    pub shortcut_builtin: SharedString,
    pub cmd_next_file: SharedString,
    pub cmd_previous_file: SharedString,
    pub cmd_toggle_viewed: SharedString,
    pub cmd_focus_tree: SharedString,
    pub cmd_focus_diff: SharedString,
    pub cmd_select_next: SharedString,
    pub cmd_select_previous: SharedString,
    pub cmd_collapse: SharedString,
    pub cmd_expand: SharedString,
    pub cmd_open_selected: SharedString,
    pub cmd_tree_toggle_viewed: SharedString,
    pub cmd_add_repositories: SharedString,
    pub cmd_next_tab: SharedString,
    pub cmd_previous_tab: SharedString,
    pub shortcuts: SharedString,
    pub shortcuts_review: SharedString,
    pub shortcuts_tree: SharedString,
    pub shortcuts_window: SharedString,
    pub shortcuts_desc: SharedString,
    pub where_review: SharedString,
    pub where_tree: SharedString,
    pub where_window: SharedString,
    pub shortcut_label: SharedString,
    pub shortcut_recording: SharedString,
    pub shortcut_none: SharedString,
    pub shortcut_record: SharedString,
    pub shortcut_hint: SharedString,
    pub add_shortcut: SharedString,
    pub reset_shortcut: SharedString,
    pub reset_all_shortcuts: SharedString,
    pub remove: SharedString,
    pub save: SharedString,
    pub auto_mark_label: SharedString,
    pub hidden_files_title: SharedString,
    pub hidden_files_tooltip: SharedString,
    pub edit_hidden_files: SharedString,
    pub hide_file_here: SharedString,
    pub hide_folder_here: SharedString,
    pub all_files_hidden: SharedString,
    pub hidden_files_setting: SharedString,
    pub hidden_files_setting_desc: SharedString,
    pub hidden_files_global: SharedString,
    pub hidden_files_global_desc: SharedString,
    pub hidden_files_repo_desc: SharedString,
    pub previous_change: SharedString,
    pub next_change: SharedString,
    pub press_again_next_file: SharedString,
    pub press_again_previous_file: SharedString,
    pub no_next_change: SharedString,
    pub no_previous_change: SharedString,
    pub file_added_tag: SharedString,
    pub file_deleted_tag: SharedString,
    pub file_renamed_tag: SharedString,
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
    pub toggle_blame: SharedString,
    pub cmd_toggle_blame: SharedString,
    pub blame_you: SharedString,
    pub blame_uncommitted: SharedString,
    pub blame_failed: SharedString,
    pub just_now: SharedString,

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
    pub shortcut_conflict_t: SharedString,
    pub hidden_count_t: SharedString,
    pub hide_here_t: SharedString,
    pub hide_everywhere_t: SharedString,
    pub hidden_files_repo_t: SharedString,
    pub hidden_files_invalid_t: SharedString,
    pub renamed_from_t: SharedString,
    pub minutes_ago_t: SharedString,
    pub hours_ago_t: SharedString,
    pub days_ago_t: SharedString,
    pub months_ago_t: SharedString,
    pub years_ago_t: SharedString,
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

    pub fn reset_progress_desc(&self, count: usize) -> String {
        fill(&self.reset_progress_desc_t, &[&count.to_string()])
    }

    pub fn hidden_count(&self, count: usize) -> String {
        fill(&self.hidden_count_t, &[&count.to_string()])
    }

    pub fn hide_here(&self, pattern: &str) -> String {
        fill(&self.hide_here_t, &[pattern])
    }

    pub fn hide_everywhere(&self, pattern: &str) -> String {
        fill(&self.hide_everywhere_t, &[pattern])
    }

    pub fn hidden_files_repo(&self, repo: &str) -> String {
        fill(&self.hidden_files_repo_t, &[repo])
    }

    pub fn hidden_files_invalid(&self, patterns: &str) -> String {
        fill(&self.hidden_files_invalid_t, &[patterns])
    }

    pub fn shortcut_conflict(&self, command: &str) -> String {
        fill(&self.shortcut_conflict_t, &[command])
    }

    pub fn zed_import_failed(&self, err: &str) -> String {
        fill(&self.zed_import_failed_t, &[err])
    }

    pub fn renamed_from(&self, path: &str) -> String {
        fill(&self.renamed_from_t, &[path])
    }
    pub fn relative_time(&self, seconds: i64) -> String {
        const MINUTE: i64 = 60;
        const HOUR: i64 = 60 * MINUTE;
        const DAY: i64 = 24 * HOUR;
        const MONTH: i64 = 30 * DAY;
        const YEAR: i64 = 365 * DAY;
        let (template, unit) = if seconds < MINUTE {
            return self.just_now.to_string();
        } else if seconds < HOUR {
            (&self.minutes_ago_t, MINUTE)
        } else if seconds < DAY {
            (&self.hours_ago_t, HOUR)
        } else if seconds < MONTH {
            (&self.days_ago_t, DAY)
        } else if seconds < YEAR {
            (&self.months_ago_t, MONTH)
        } else {
            (&self.years_ago_t, YEAR)
        };
        let count = seconds / unit;
        plural(template, count.unsigned_abs())
    }
}

fn plural(template: &str, count: u64) -> String {
    let forms: Vec<&str> = template.split('|').collect();
    let form = if forms.len() == 3 {
        match (count % 10, count % 100) {
            (1, rest) if rest != 11 => 0,
            (2..=4, rest) if !(12..=14).contains(&rest) => 1,
            _ => 2,
        }
    } else {
        usize::from(count != 1)
    };
    fill(
        forms
            .get(form)
            .or(forms.last())
            .copied()
            .unwrap_or_default(),
        &[&count.to_string()],
    )
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
    fn relative_time_picks_the_unit_and_plural_form() {
        let en = Lang::En.strings();
        let ru = Lang::Ru.strings();
        let day = 24 * 3600;
        assert_eq!(en.relative_time(5), "just now");
        assert_eq!(en.relative_time(-30), "just now");
        assert_eq!(en.relative_time(60), "1 minute ago");
        assert_eq!(en.relative_time(3 * 3600 + 5), "3 hours ago");
        assert_eq!(en.relative_time(day), "1 day ago");
        assert_eq!(en.relative_time(45 * day), "1 month ago");
        assert_eq!(en.relative_time(800 * day), "2 years ago");
        assert_eq!(ru.relative_time(21 * 60), "21 минуту назад");
        assert_eq!(ru.relative_time(3 * 3600), "3 часа назад");
        assert_eq!(ru.relative_time(12 * 3600), "12 часов назад");
        assert_eq!(ru.relative_time(22 * day), "22 дня назад");
        assert_eq!(ru.relative_time(11 * 30 * day), "11 месяцев назад");
        assert_eq!(ru.relative_time(5 * 365 * day), "5 лет назад");
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
                &s.shortcut_conflict_t,
                &s.hidden_count_t,
                &s.hide_here_t,
                &s.hide_everywhere_t,
                &s.hidden_files_repo_t,
                &s.hidden_files_invalid_t,
                &s.renamed_from_t,
                &s.reset_progress_desc_t,
            ] {
                assert_eq!(template.matches("{}").count(), 1, "{lang:?}: {template}");
            }
            assert_eq!(s.merge_base_t.matches("{}").count(), 2, "{lang:?}");
            for template in [
                &s.minutes_ago_t,
                &s.hours_ago_t,
                &s.days_ago_t,
                &s.months_ago_t,
                &s.years_ago_t,
            ] {
                let forms: Vec<&str> = template.split('|').collect();
                assert!(matches!(forms.len(), 2 | 3), "{lang:?}: {template}");
                for form in forms {
                    assert_eq!(form.matches("{}").count(), 1, "{lang:?}: {template}");
                }
            }
            assert_eq!(s.viewed_progress_t.matches("{}").count(), 2, "{lang:?}");
        }
    }
}
