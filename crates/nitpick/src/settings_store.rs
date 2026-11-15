use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge::ForgeKind;
use gpui::{App, Global, px};
use gpui_component::{
    Theme, ThemeRegistry, WindowExt as _, highlighter::HighlightTheme, notification::Notification,
};
use serde::{Deserialize, Serialize};

use crate::localization::tr;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Named(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LangChoice {
    #[default]
    System,
    Named(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeSide {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeMode {
    #[default]
    Tree,
    List,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLayout {
    #[default]
    Split,
    Unified,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapLines {
    #[default]
    SplitOnly,
    Always,
    Never,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxPalettes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<String>,
}

impl SyntaxPalettes {
    pub fn get(&self, dark: bool) -> Option<&String> {
        if dark {
            self.dark.as_ref()
        } else {
            self.light.as_ref()
        }
    }

    pub fn set(&mut self, dark: bool, palette: Option<String>) {
        if dark {
            self.dark = palette;
        } else {
            self.light = palette;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(default)]
    pub language: LangChoice,
    #[serde(default)]
    pub tree_side: TreeSide,
    #[serde(default)]
    pub diff_layout: DiffLayout,
    #[serde(default)]
    pub ignore_whitespace: bool,
    #[serde(default = "default_true")]
    pub advance_on_viewed: bool,
    #[serde(default)]
    pub auto_mark_viewed: bool,
    #[serde(default = "default_true")]
    pub collapse_viewed_folders: bool,
    #[serde(default = "default_true")]
    pub tree_comments: bool,
    #[serde(default)]
    pub strike_viewed: bool,
    #[serde(default = "default_tree_width")]
    pub tree_width: f32,
    #[serde(default = "default_split_ratio")]
    pub split_ratio: f32,
    #[serde(default)]
    pub hidden_files: Vec<String>,
    #[serde(default)]
    pub repo_hidden_files: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub keybindings: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub tree_mode: TreeMode,
    #[serde(default)]
    pub show_hidden_files: bool,
    #[serde(default = "default_true")]
    pub compact_folders: bool,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: u16,
    #[serde(default)]
    pub code_zoom: i16,
    #[serde(default)]
    pub gitlab_url: String,
    #[serde(default, with = "secret_store::serde_string")]
    pub gitlab_token: String,
    #[serde(default)]
    pub github_url: String,
    #[serde(default, with = "secret_store::serde_string")]
    pub github_token: String,
    #[serde(default)]
    pub syntax_colors: SyntaxPalettes,
    #[serde(default)]
    pub show_blame: bool,
    #[serde(default)]
    pub show_whole_file: bool,
    #[serde(default)]
    pub wrap_lines: WrapLines,
    #[serde(default)]
    pub editor_command: String,
}

pub const UI_SCALE_RANGE: (u16, u16) = (80, 150);
pub const UI_SCALES: [u16; 6] = [90, 100, 110, 125, 140, 150];
pub const CODE_ZOOM_RANGE: (i16, i16) = (-5, 14);

fn default_ui_scale() -> u16 {
    100
}

fn default_true() -> bool {
    true
}

pub const TREE_WIDTH_RANGE: (f32, f32) = (160., 720.);

fn default_tree_width() -> f32 {
    300.
}

fn default_split_ratio() -> f32 {
    0.5
}

pub const SPLIT_RATIO_RANGE: (f32, f32) = (0.1, 0.9);

pub const SPLIT_MIN_COLUMN: f32 = 160.;

pub fn fit_split_ratio(ratio: f32, width: f32) -> f32 {
    if !ratio.is_finite() || !width.is_finite() || width < 2. * SPLIT_MIN_COLUMN {
        return 0.5;
    }
    let edge = SPLIT_MIN_COLUMN / width;
    ratio.clamp(
        edge.max(SPLIT_RATIO_RANGE.0),
        (1. - edge).min(SPLIT_RATIO_RANGE.1),
    )
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            language: LangChoice::default(),
            tree_side: TreeSide::default(),
            diff_layout: DiffLayout::default(),
            ignore_whitespace: false,
            advance_on_viewed: true,
            auto_mark_viewed: false,
            collapse_viewed_folders: true,
            tree_comments: true,
            strike_viewed: false,
            tree_width: default_tree_width(),
            split_ratio: default_split_ratio(),
            hidden_files: Vec::new(),
            repo_hidden_files: BTreeMap::new(),
            keybindings: BTreeMap::new(),
            tree_mode: TreeMode::Tree,
            show_hidden_files: false,
            compact_folders: true,
            ui_scale: default_ui_scale(),
            code_zoom: 0,
            gitlab_url: String::new(),
            gitlab_token: String::new(),
            github_url: String::new(),
            github_token: String::new(),
            syntax_colors: SyntaxPalettes::default(),
            show_blame: false,
            show_whole_file: false,
            wrap_lines: WrapLines::default(),
            editor_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HideScope {
    Everywhere,
    Repository(PathBuf),
}

pub fn repo_key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

pub fn add_hide_pattern(cx: &mut App, scope: HideScope, pattern: String) {
    update_settings(cx, move |s| {
        let list = match &scope {
            HideScope::Everywhere => &mut s.hidden_files,
            HideScope::Repository(root) => s.repo_hidden_files.entry(repo_key(root)).or_default(),
        };
        if !list.contains(&pattern) {
            list.push(pattern);
        }
    });
}

impl Settings {
    pub fn repo_hidden(&self, root: &Path) -> &[String] {
        self.repo_hidden_files
            .get(&repo_key(root))
            .map_or(&[], Vec::as_slice)
    }

    pub fn forge_url(&self, kind: ForgeKind) -> String {
        kind.web_url(match kind {
            ForgeKind::GitLab => &self.gitlab_url,
            ForgeKind::GitHub => &self.github_url,
        })
    }

    pub fn forge_token(&self, kind: ForgeKind) -> &str {
        match kind {
            ForgeKind::GitLab => self.gitlab_token.trim(),
            ForgeKind::GitHub => self.github_token.trim(),
        }
    }

    pub fn forge_configured(&self, kind: ForgeKind) -> bool {
        !self.forge_url(kind).is_empty() && !self.forge_token(kind).is_empty()
    }

    pub fn forge_hosts(&self) -> Vec<(ForgeKind, String)> {
        [ForgeKind::GitLab, ForgeKind::GitHub]
            .into_iter()
            .map(|kind| (kind, self.forge_url(kind)))
            .filter(|(_, url)| !url.is_empty())
            .collect()
    }

    pub fn forge_client(&self, kind: ForgeKind) -> Option<forge::Client> {
        if !self.forge_configured(kind) {
            return None;
        }
        match forge::Client::new(kind, &self.forge_url(kind), self.forge_token(kind)) {
            Ok(client) => Some(client),
            Err(err) => {
                log::warn!("{} client: {err}", kind.name());
                None
            }
        }
    }
}

pub struct SettingsStore {
    settings: Settings,
    path: PathBuf,
}

impl Global for SettingsStore {}

pub fn app_dir(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(|| PathBuf::from(".")).join("nitpick")
}

impl SettingsStore {
    pub fn load() -> Self {
        let config = app_dir(dirs::config_dir());
        secret_store::init(&config);
        Self::load_from(config.join("settings.json"))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let (settings, reseal) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                match secret_store::watch_reseal(|| serde_json::from_str::<Settings>(&text)) {
                    (Ok(settings), reseal) => (settings, reseal),
                    (Err(err), _) => {
                        log::warn!(
                            "settings: {} is unreadable, using defaults: {err}",
                            path.display()
                        );
                        (Settings::default(), false)
                    }
                }
            }
            Err(_) => (Settings::default(), false),
        };
        let store = Self { settings, path };
        if reseal && let Err(err) = store.save() {
            log::warn!("settings: reseal failed: {err}");
        }
        store
    }

    pub fn get(&self) -> &Settings {
        &self.settings
    }

    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) -> anyhow::Result<()> {
        change(&mut self.settings);
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.settings)?;
        review_core::write_atomic(&self.path, &bytes)?;
        Ok(())
    }
}

pub fn settings(cx: &App) -> &Settings {
    cx.global::<SettingsStore>().get()
}

pub fn update_settings(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    if let Err(err) = cx.global_mut::<SettingsStore>().update(change) {
        log::error!("settings: save failed: {err}");
        let message = tr().settings_save_failed(&err.to_string());
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    window.push_notification(Notification::error(message), cx);
                });
            });
        }
    }
    cx.refresh_windows();
}

fn reset_theme_metrics(cx: &mut App) {
    let defaults = Theme::default();
    Theme::update(cx, |theme| {
        theme.font_size = defaults.font_size;
        theme.font_family = defaults.font_family.clone();
        theme.mono_font_size = defaults.mono_font_size;
        theme.mono_font_family = defaults.mono_font_family.clone();
        theme.radius = defaults.radius;
        theme.radius_lg = defaults.radius_lg;
        theme.window_radius = defaults.window_radius;
        theme.shadow = defaults.shadow;
    });
}

fn apply_zoom(cx: &mut App) {
    let (scale, code) = {
        let current = settings(cx);
        (current.ui_scale, current.code_zoom)
    };
    Theme::update(cx, |theme| {
        let (min, max) = UI_SCALE_RANGE;
        theme.font_size *= scale.clamp(min, max) as f32 / 100.;
        let (min, max) = CODE_ZOOM_RANGE;
        let mono = f32::from(theme.mono_font_size) + code.clamp(min, max) as f32;
        theme.mono_font_size = px(mono.max(6.));
    });
}

const SYSTEM_LIGHT_THEME: &str = "One Light";
const SYSTEM_DARK_THEME: &str = "One Dark";

pub fn apply_theme(choice: &ThemeChoice, cx: &mut App) {
    reset_theme_metrics(cx);
    match choice {
        ThemeChoice::System => {
            let registry = ThemeRegistry::global(cx);
            let light = registry.themes().get(SYSTEM_LIGHT_THEME).cloned();
            let dark = registry.themes().get(SYSTEM_DARK_THEME).cloned();
            let theme = cx.global_mut::<Theme>();
            if let Some(light) = light {
                theme.light_theme = light;
            }
            if let Some(dark) = dark {
                theme.dark_theme = dark;
            }
            Theme::sync_system_appearance(None, cx);
        }
        ThemeChoice::Named(name) => {
            let config = ThemeRegistry::global(cx)
                .themes()
                .get(name.as_str())
                .cloned();
            match config {
                Some(config) => {
                    let mode = config.mode;
                    let theme = cx.global_mut::<Theme>();
                    if mode.is_dark() {
                        theme.dark_theme = config;
                    } else {
                        theme.light_theme = config;
                    }
                    Theme::change(mode, None, cx);
                }
                None => {
                    let mode = Theme::global(cx).mode;
                    Theme::change(mode, None, cx);
                }
            }
        }
    }
    apply_zoom(cx);
    cx.refresh_windows();
}

pub fn apply_saved_theme(cx: &mut App) {
    let choice = settings(cx).theme.clone();
    apply_theme(&choice, cx);
}

#[derive(Default)]
struct SyntaxPreview(Option<Option<String>>);

impl Global for SyntaxPreview {}

pub fn observe_theme_for_syntax_colors(cx: &mut App) {
    cx.set_global(SyntaxPreview::default());
    cx.observe_global::<Theme>(apply_syntax_colors).detach();
}

pub fn preview_syntax_colors(palette: Option<String>, cx: &mut App) {
    cx.set_global(SyntaxPreview(Some(palette)));
    apply_syntax_colors(cx);
}

pub fn end_syntax_preview(cx: &mut App) {
    cx.set_global(SyntaxPreview(None));
    apply_syntax_colors(cx);
}

pub fn syntax_palette_names(dark: bool, cx: &App) -> Vec<String> {
    let mut names: Vec<String> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .filter(|config| config.mode.is_dark() == dark && config.highlight.is_some())
        .map(|config| config.name.to_string())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

pub fn apply_syntax_colors(cx: &mut App) {
    let theme = Theme::global(cx);
    let dark = theme.mode.is_dark();
    let chosen = match cx.try_global::<SyntaxPreview>().and_then(|p| p.0.as_ref()) {
        Some(previewed) => previewed.as_ref(),
        None => settings(cx).syntax_colors.get(dark),
    };
    let palette = chosen
        .and_then(|name| ThemeRegistry::global(cx).themes().get(name.as_str()))
        .filter(|config| config.mode.is_dark() == dark)
        .and_then(|config| config.highlight.as_ref());
    let own = if dark {
        &theme.dark_theme
    } else {
        &theme.light_theme
    };
    let fallback = if dark {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
    };
    let syntax = &palette
        .or(own.highlight.as_ref())
        .unwrap_or(&fallback.style)
        .syntax;
    if &theme.highlight_theme.style.syntax == syntax {
        return;
    }
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.syntax = syntax.clone();
    let highlight = Arc::new(highlight);
    Theme::update(cx, |theme| theme.highlight_theme = highlight);
}

#[cfg(test)]
#[path = "settings_store_tests.rs"]
mod tests;
