use std::path::{Path, PathBuf};

use gpui::{App, Global};
use gpui_component::{Theme, ThemeRegistry, WindowExt as _, notification::Notification};
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
pub enum DiffLayout {
    #[default]
    Split,
    Unified,
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
    pub gitlab_url: String,
    #[serde(default, with = "secret_store::serde_string")]
    pub gitlab_token: String,
}

fn default_true() -> bool {
    true
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
            gitlab_url: String::new(),
            gitlab_token: String::new(),
        }
    }
}

impl Settings {
    pub fn gitlab_configured(&self) -> bool {
        !self.gitlab_url.trim().is_empty() && !self.gitlab_token.trim().is_empty()
    }

    pub fn gitlab_client(&self) -> Option<gitlab::Client> {
        if !self.gitlab_configured() {
            return None;
        }
        match gitlab::Client::new(self.gitlab_url.trim(), self.gitlab_token.trim()) {
            Ok(client) => Some(client),
            Err(err) => {
                log::warn!("gitlab client: {err}");
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
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("sloptaster")
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
        write_private(&self.path, &bytes)?;
        Ok(())
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    review_core::write_atomic(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
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

pub fn apply_theme(choice: &ThemeChoice, cx: &mut App) {
    match choice {
        ThemeChoice::System => Theme::sync_system_appearance(None, cx),
        ThemeChoice::Named(name) => {
            let Some(config) = ThemeRegistry::global(cx)
                .themes()
                .get(name.as_str())
                .cloned()
            else {
                return;
            };
            let mode = config.mode;
            let theme = cx.global_mut::<Theme>();
            if mode.is_dark() {
                theme.dark_theme = config;
            } else {
                theme.light_theme = config;
            }
            Theme::change(mode, None, cx);
        }
    }
    cx.refresh_windows();
}

pub fn apply_saved_theme(cx: &mut App) {
    let choice = settings(cx).theme.clone();
    apply_theme(&choice, cx);
}
