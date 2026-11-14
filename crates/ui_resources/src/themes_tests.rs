use std::collections::HashSet;

use gpui::{FontStyle, FontWeight, Rgba};
use gpui_component::ThemeSet;

use super::import_zed_theme;

const ZED_FAMILY: &str = r##"{
  "$schema": "https://zed.dev/schema/themes/v0.2.0.json",
  "name": "Snazzy",
  "author": "eivs",
  "themes": [
    {
      "name": "Snazzy Dark",
      "appearance": "dark",
      "style": {
        "background.appearance": "blurred",
        "editor.background": "#282a36ff",
        "editor.foreground": "#eff0ebff",
        "text": "#eff0ebff",
        "text.muted": "#a5a5a9ff",
        "text.accent": "#57c7ffff",
        "border": "#3a3c4eff",
        "element.background": null,
        "created": "#5af78eff",
        "deleted": "#ff5c57ff",
        "hint": "#9aedfeff",
        "players": [{ "cursor": "#57c7ffff", "selection": "#57c7ff3d" }],
        "syntax": {
          "keyword": { "color": "#ff6ac1ff", "font_style": null, "font_weight": null },
          "comment": { "color": "#616263ff", "font_style": "italic", "font_weight": null },
          "title": { "color": "#57c7ffff", "font_style": null, "font_weight": 700 },
          "string.escape": { "color": "#57c7ffff", "font_style": "oblique", "font_weight": 450 },
          "concept": { "color": null, "font_style": null, "font_weight": null }
        }
      }
    },
    {
      "name": "Snazzy Light",
      "appearance": "light",
      "style": { "editor.background": "#fafafa", "text": "#333", "syntax": {} }
    }
  ]
}"##;

#[test]
fn imported_zed_family_loads_as_a_theme_set() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("snazzy.json");
    std::fs::write(&source, ZED_FAMILY).unwrap();
    let themes_dir = dir.path().join("themes");
    let registered = HashSet::from(["Snazzy Light".to_string()]);

    let converted = import_zed_theme(&source, &themes_dir, &registered).unwrap();
    let names: Vec<&str> = converted.themes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["Snazzy Dark", "Snazzy Light (Zed)"]);

    let text = std::fs::read_to_string(themes_dir.join("zed-snazzy.json")).unwrap();
    let set: ThemeSet = serde_json::from_str(&text).unwrap();
    assert_eq!(set.themes.len(), 2);
    let dark = &set.themes[0];
    assert!(dark.mode.is_dark());
    assert!(!set.themes[1].mode.is_dark());
    let highlight = dark.highlight.as_ref().unwrap();
    let comment = highlight.syntax.style("comment").unwrap();
    assert_eq!(comment.font_style, Some(FontStyle::Italic));
    let title = highlight.syntax.style("title").unwrap();
    assert_eq!(title.font_weight, Some(FontWeight::BOLD));
    let escape = highlight.syntax.style("string.escape").unwrap();
    assert_eq!(escape.font_style, Some(FontStyle::Italic));
    assert_eq!(escape.font_weight, Some(FontWeight::MEDIUM));
    assert!(highlight.syntax.style("keyword").unwrap().color.is_some());

    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    for theme in raw["themes"].as_array().unwrap() {
        for (key, color) in theme["colors"].as_object().unwrap() {
            let color = color.as_str().unwrap();
            assert!(Rgba::try_from(color).is_ok(), "{key}: {color}");
        }
    }
}

#[test]
fn reimporting_keeps_the_names_the_file_already_has() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("snazzy.json");
    std::fs::write(&source, ZED_FAMILY).unwrap();
    let themes_dir = dir.path().join("themes");
    let first = import_zed_theme(&source, &themes_dir, &HashSet::new()).unwrap();
    let registered: HashSet<String> = first.themes.iter().map(|t| t.name.clone()).collect();
    let again = import_zed_theme(&source, &themes_dir, &registered).unwrap();
    assert_eq!(again.themes, first.themes);
}

#[test]
fn a_file_that_is_not_a_zed_theme_is_an_error_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("package.json");
    std::fs::write(&source, r#"{ "name": "app" }"#).unwrap();
    let themes_dir = dir.path().join("themes");
    let err = import_zed_theme(&source, &themes_dir, &HashSet::new()).unwrap_err();
    assert!(err.to_string().contains("not a Zed theme"), "{err}");
    assert!(!themes_dir.exists());
}
