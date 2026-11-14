mod jsonc;

use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Zed theme file: it has no \"themes\" list")]
    NotZedTheme,
    #[error("the file defines no themes")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedTheme {
    pub name: String,
    pub dark: bool,
}

#[derive(Debug, Clone)]
pub struct Converted {
    pub family: String,
    pub file_stem: String,
    pub themes: Vec<ConvertedTheme>,
    pub json: String,
}

const UI_COLORS: &[(&str, &[&str])] = &[
    ("background", &["editor.background", "background"]),
    ("foreground", &["text", "editor.foreground"]),
    ("border", &["border"]),
    ("input.border", &["border"]),
    ("window.border", &["border"]),
    ("ring", &["border.focused"]),
    (
        "muted.background",
        &["surface.background", "panel.background"],
    ),
    ("muted.foreground", &["text.muted"]),
    (
        "accent.background",
        &["ghost_element.hover", "element.hover"],
    ),
    ("accent.foreground", &["text"]),
    ("primary.background", &["text.accent", "icon.accent"]),
    ("primary.foreground", &["editor.background", "background"]),
    ("secondary.background", &["element.background"]),
    ("secondary.hover.background", &["element.hover"]),
    ("secondary.active.background", &["element.active"]),
    ("secondary.foreground", &["text"]),
    ("popover.background", &["elevated_surface.background"]),
    ("popover.foreground", &["text"]),
    (
        "list.active.background",
        &["ghost_element.selected", "element.selected"],
    ),
    ("list.active.border", &["border.selected", "border.focused"]),
    (
        "list.hover.background",
        &["ghost_element.hover", "element.hover"],
    ),
    ("sidebar.background", &["panel.background"]),
    ("sidebar.foreground", &["text"]),
    ("sidebar.border", &["border"]),
    (
        "title_bar.background",
        &["title_bar.background", "background"],
    ),
    ("title_bar.border", &["border"]),
    ("tab_bar.background", &["tab_bar.background"]),
    ("tab.background", &["tab.inactive_background"]),
    ("tab.active.background", &["tab.active_background"]),
    ("tab.foreground", &["text.muted"]),
    ("tab.active.foreground", &["text"]),
    ("status_bar.background", &["status_bar.background"]),
    ("status_bar.border", &["border"]),
    ("scrollbar.background", &["scrollbar.track.background"]),
    (
        "scrollbar.thumb.background",
        &["scrollbar.thumb.background"],
    ),
    (
        "scrollbar.thumb.hover.background",
        &["scrollbar.thumb.hover_background"],
    ),
    ("link", &["text.accent"]),
    ("link.hover", &["link_text.hover"]),
    ("drop_target.background", &["drop_target.background"]),
    (
        "success.background",
        &["version_control.added", "created", "success"],
    ),
    (
        "danger.background",
        &["version_control.deleted", "deleted", "error"],
    ),
    ("warning.background", &["warning"]),
    ("info.background", &["info"]),
    ("base.red", &["terminal.ansi.red"]),
    ("base.red.light", &["terminal.ansi.bright_red"]),
    ("base.green", &["terminal.ansi.green"]),
    ("base.green.light", &["terminal.ansi.bright_green"]),
    ("base.blue", &["terminal.ansi.blue"]),
    ("base.blue.light", &["terminal.ansi.bright_blue"]),
    ("base.yellow", &["terminal.ansi.yellow"]),
    ("base.yellow.light", &["terminal.ansi.bright_yellow"]),
    ("base.magenta", &["terminal.ansi.magenta"]),
    ("base.magenta.light", &["terminal.ansi.bright_magenta"]),
    ("base.cyan", &["terminal.ansi.cyan"]),
    ("base.cyan.light", &["terminal.ansi.bright_cyan"]),
];

const HIGHLIGHT_KEYS: &[&str] = &[
    "editor.background",
    "editor.foreground",
    "editor.active_line.background",
    "editor.line_number",
    "editor.active_line_number",
    "editor.invisible",
    "editor.gutter.background",
    "error",
    "error.background",
    "error.border",
    "warning",
    "warning.background",
    "warning.border",
    "info",
    "info.background",
    "info.border",
    "success",
    "success.background",
    "success.border",
    "hint",
    "hint.background",
    "hint.border",
    "created",
    "modified",
    "deleted",
];

pub fn convert(source: &str, is_taken: impl Fn(&str) -> bool) -> Result<Converted, ConvertError> {
    let root = parse_lenient(source)?;
    let themes = root
        .get("themes")
        .and_then(Value::as_array)
        .ok_or(ConvertError::NotZedTheme)?;
    let family = non_empty(root.get("name"))
        .unwrap_or("Zed theme")
        .to_string();
    let mut converted = Vec::new();
    let mut summary: Vec<ConvertedTheme> = Vec::new();
    for theme in themes {
        let Some(style) = theme.get("style").and_then(Value::as_object) else {
            continue;
        };
        let dark = theme.get("appearance").and_then(Value::as_str) != Some("light");
        let base = non_empty(theme.get("name")).unwrap_or(&family);
        let name = unique_name(base, |candidate| {
            is_taken(candidate) || summary.iter().any(|t| t.name == candidate)
        });
        let mut config = Map::new();
        config.insert("name".into(), Value::from(name.clone()));
        config.insert(
            "mode".into(),
            Value::from(if dark { "dark" } else { "light" }),
        );
        config.insert("colors".into(), Value::Object(ui_colors(style)));
        config.insert("highlight".into(), Value::Object(highlight(style)));
        converted.push(Value::Object(config));
        summary.push(ConvertedTheme { name, dark });
    }
    if converted.is_empty() {
        return Err(ConvertError::Empty);
    }
    let mut set = Map::new();
    set.insert("name".into(), Value::from(family.clone()));
    if let Some(author) = non_empty(root.get("author")) {
        set.insert("author".into(), Value::from(author));
    }
    set.insert("themes".into(), Value::Array(converted));
    Ok(Converted {
        file_stem: format!("zed-{}", slug(&family)),
        family,
        themes: summary,
        json: serde_json::to_string_pretty(&Value::Object(set))?,
    })
}

fn parse_lenient(source: &str) -> Result<Value, ConvertError> {
    match serde_json::from_str(source) {
        Ok(value) => Ok(value),
        Err(err) => serde_json::from_str(&jsonc::strip(source)).map_err(|_| err.into()),
    }
}

fn non_empty(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn unique_name(base: &str, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(base) {
        return base.to_string();
    }
    let first = format!("{base} (Zed)");
    if !is_taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{base} (Zed {n})"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or(first)
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "theme".to_string()
    } else {
        trimmed.to_string()
    }
}

fn ui_colors(style: &Map<String, Value>) -> Map<String, Value> {
    let mut colors = Map::new();
    for (key, sources) in UI_COLORS {
        if let Some(color) = sources.iter().find_map(|s| style.get(*s).and_then(color)) {
            colors.insert((*key).into(), Value::from(color));
        }
    }
    let player = style
        .get("players")
        .and_then(Value::as_array)
        .and_then(|players| players.first());
    if let Some(player) = player {
        if let Some(cursor) = player.get("cursor").and_then(color) {
            colors.insert("caret".into(), Value::from(cursor));
        }
        if let Some(selection) = player.get("selection").and_then(color) {
            colors.insert("selection.background".into(), Value::from(selection));
        }
    }
    colors
}

fn highlight(style: &Map<String, Value>) -> Map<String, Value> {
    let mut highlight = Map::new();
    for key in HIGHLIGHT_KEYS {
        if let Some(color) = style.get(*key).and_then(color) {
            highlight.insert((*key).into(), Value::from(color));
        }
    }
    let syntax: Map<String, Value> = style
        .get("syntax")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(capture, value)| Some((capture.clone(), syntax_style(value)?)))
        .collect();
    highlight.insert("syntax".into(), Value::Object(syntax));
    highlight
}

fn syntax_style(value: &Value) -> Option<Value> {
    let style = value.as_object()?;
    let mut out = Map::new();
    if let Some(color) = style.get("color").and_then(color) {
        out.insert("color".into(), Value::from(color));
    }
    if let Some(font_style) = style
        .get("font_style")
        .and_then(Value::as_str)
        .and_then(font_style)
    {
        out.insert("font_style".into(), Value::from(font_style));
    }
    if let Some(weight) = style
        .get("font_weight")
        .and_then(Value::as_f64)
        .and_then(font_weight)
    {
        out.insert("font_weight".into(), Value::from(weight));
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

pub fn color(value: &Value) -> Option<String> {
    let hex = value.as_str()?.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut digits: String = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => hex.to_string(),
        _ => return None,
    };
    digits.make_ascii_lowercase();
    if digits.len() == 6 {
        digits.push_str("ff");
    }
    Some(format!("#{digits}"))
}

pub fn font_style(style: &str) -> Option<&'static str> {
    match style {
        "normal" => Some("normal"),
        "italic" | "oblique" => Some("italic"),
        "underline" => Some("underline"),
        _ => None,
    }
}

pub fn font_weight(weight: f64) -> Option<u16> {
    if !weight.is_finite() {
        return None;
    }
    let hundreds = (weight / 100.0).round().clamp(1.0, 9.0) as u16;
    Some(hundreds * 100)
}

#[cfg(test)]
mod tests;
