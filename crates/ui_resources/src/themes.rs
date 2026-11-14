use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use gpui::App;
use gpui_component::theme::ThemeRegistry;

const BUNDLED_THEMES: &[(&str, &str)] = &[
    ("adventure.json", include_str!("../themes/adventure.json")),
    ("alduin.json", include_str!("../themes/alduin.json")),
    ("asciinema.json", include_str!("../themes/asciinema.json")),
    ("ayu.json", include_str!("../themes/ayu.json")),
    ("catppuccin.json", include_str!("../themes/catppuccin.json")),
    ("everforest.json", include_str!("../themes/everforest.json")),
    ("fahrenheit.json", include_str!("../themes/fahrenheit.json")),
    ("flexoki.json", include_str!("../themes/flexoki.json")),
    ("gruvbox.json", include_str!("../themes/gruvbox.json")),
    ("harper.json", include_str!("../themes/harper.json")),
    ("hybrid.json", include_str!("../themes/hybrid.json")),
    ("jellybeans.json", include_str!("../themes/jellybeans.json")),
    ("kibble.json", include_str!("../themes/kibble.json")),
    (
        "macos-classic.json",
        include_str!("../themes/macos-classic.json"),
    ),
    ("matrix.json", include_str!("../themes/matrix.json")),
    (
        "mellifluous.json",
        include_str!("../themes/mellifluous.json"),
    ),
    ("molokai.json", include_str!("../themes/molokai.json")),
    ("solarized.json", include_str!("../themes/solarized.json")),
    ("spaceduck.json", include_str!("../themes/spaceduck.json")),
    ("tokyonight.json", include_str!("../themes/tokyonight.json")),
    ("twilight.json", include_str!("../themes/twilight.json")),
];

pub fn themes_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("slopcop").join("themes"))
}

fn stage_bundled_themes() -> anyhow::Result<PathBuf> {
    let dir = themes_dir().ok_or_else(|| anyhow::anyhow!("data dir unavailable"))?;
    std::fs::create_dir_all(&dir)?;
    for (name, body) in BUNDLED_THEMES {
        let path = dir.join(name);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(dir)
}

/// Write bundled theme JSON files to disk and register them with `ThemeRegistry`.
/// `on_loaded` is called after the initial load completes.
pub fn register_bundled_themes<F: Fn(&mut App) + 'static>(cx: &mut App, on_loaded: F) {
    match stage_bundled_themes() {
        Ok(dir) => {
            if let Err(e) = ThemeRegistry::watch_dir(dir, cx, on_loaded) {
                log::warn!("failed to register bundled themes: {e}");
            }
        }
        Err(e) => log::warn!("failed to stage bundled themes: {e}"),
    }
}

pub struct ZedThemeFile {
    pub path: PathBuf,
    pub label: String,
}

pub fn zed_theme_files() -> Vec<ZedThemeFile> {
    let mut files = Vec::new();
    if let Some(installed) = zed_data_dir().map(|d| d.join("extensions").join("installed"))
        && let Ok(extensions) = std::fs::read_dir(installed)
    {
        for extension in extensions.flatten() {
            let name = extension.file_name().to_string_lossy().into_owned();
            for path in json_files(&extension.path().join("themes")) {
                let label = format!("{name}/{}", file_name(&path));
                files.push(ZedThemeFile { path, label });
            }
        }
    }
    if let Some(dir) = zed_user_themes_dir() {
        for path in json_files(&dir) {
            let label = file_name(&path);
            files.push(ZedThemeFile { path, label });
        }
    }
    files.sort_by_key(|file| file.label.to_lowercase());
    files
}

fn zed_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::data_local_dir().map(|d| d.join("Zed"))
    } else if cfg!(target_os = "macos") {
        dirs::data_dir().map(|d| d.join("Zed"))
    } else {
        dirs::data_dir().map(|d| d.join("zed"))
    }
}

fn zed_user_themes_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::config_dir().map(|d| d.join("Zed").join("themes"))
    } else if cfg!(target_os = "macos") {
        dirs::home_dir().map(|d| d.join(".config").join("zed").join("themes"))
    } else {
        dirs::config_dir().map(|d| d.join("zed").join("themes"))
    }
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "json"))
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn import_zed_theme(
    source: &Path,
    themes_dir: &Path,
    registered: &HashSet<String>,
) -> anyhow::Result<zed_theme::Converted> {
    let text =
        std::fs::read_to_string(source).with_context(|| format!("reading {}", source.display()))?;
    let file_stem = zed_theme::convert(&text, |_| false)?.file_stem;
    let target = themes_dir.join(format!("{file_stem}.json"));
    let replaced = theme_names(&target);
    let converted = zed_theme::convert(&text, |name| {
        registered.contains(name) && !replaced.contains(name)
    })?;
    std::fs::create_dir_all(themes_dir)
        .with_context(|| format!("creating {}", themes_dir.display()))?;
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, &converted.json)
        .and_then(|()| std::fs::rename(&tmp, &target))
        .with_context(|| format!("writing {}", target.display()))?;
    Ok(converted)
}

fn theme_names(path: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    let Ok(set) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashSet::new();
    };
    set.get("themes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|theme| theme.get("name")?.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
#[path = "themes_tests.rs"]
mod tests;
