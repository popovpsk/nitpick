use rstest::rstest;
use serde_json::{Value, json};

use crate::*;

fn family() -> Value {
    json!({
        "$schema": "https://zed.dev/schema/themes/v0.2.0.json",
        "name": "Snazzy Theme!",
        "author": "eivs",
        "themes": [
            {
                "name": "Snazzy Dark",
                "appearance": "dark",
                "style": {
                    "background": "#282a36ff",
                    "editor.background": "#1e1f29",
                    "editor.foreground": "#eff0eb",
                    "editor.line_number": "#5c5d5fff",
                    "text": "#eff0ebff",
                    "text.muted": "#a5a5a9ff",
                    "text.accent": "#57c7ffff",
                    "border": "#3a3c4eff",
                    "element.hover": "#ffffff10",
                    "ghost_element.hover": null,
                    "created": "#5af78eff",
                    "deleted": "#ff5c57ff",
                    "error": "#ff5c57ff",
                    "terminal.ansi.green": "#5af78e",
                    "players": [
                        { "cursor": "#57c7ffff", "background": "#57c7ffff", "selection": "#57c7ff3d" }
                    ],
                    "syntax": {
                        "keyword": { "color": "#ff6ac1ff", "font_style": null, "font_weight": null },
                        "comment": { "color": "#616263ff", "font_style": "oblique", "font_weight": null },
                        "title": { "color": "#57c7ff", "font_weight": 650 },
                        "string.special.path": { "color": "#f3f99dff" },
                        "unknown_hint": { "color": null, "font_style": null, "font_weight": null }
                    }
                }
            },
            {
                "name": "Snazzy Light",
                "appearance": "light",
                "style": { "editor.background": "#fafafa", "syntax": {} }
            },
            { "name": "No style here", "appearance": "dark" }
        ]
    })
}

fn convert_value(value: &Value, taken: &[&str]) -> (Converted, Value) {
    let converted = convert(&value.to_string(), |name| taken.contains(&name)).unwrap();
    let json = serde_json::from_str(&converted.json).unwrap();
    (converted, json)
}

#[test]
fn converts_a_family_to_a_theme_set() {
    let (converted, set) = convert_value(&family(), &[]);
    assert_eq!(converted.family, "Snazzy Theme!");
    assert_eq!(converted.file_stem, "zed-snazzy-theme");
    assert_eq!(
        converted.themes,
        vec![
            ConvertedTheme {
                name: "Snazzy Dark".into(),
                dark: true
            },
            ConvertedTheme {
                name: "Snazzy Light".into(),
                dark: false
            },
        ]
    );
    assert_eq!(set["name"], "Snazzy Theme!");
    assert_eq!(set["author"], "eivs");
    let dark = &set["themes"][0];
    assert_eq!(dark["mode"], "dark");
    assert_eq!(set["themes"][1]["mode"], "light");
    assert_eq!(set["themes"].as_array().unwrap().len(), 2);

    let colors = &dark["colors"];
    assert_eq!(colors["background"], "#1e1f29ff");
    assert_eq!(colors["foreground"], "#eff0ebff");
    assert_eq!(colors["muted.foreground"], "#a5a5a9ff");
    assert_eq!(colors["primary.background"], "#57c7ffff");
    assert_eq!(colors["accent.background"], "#ffffff10");
    assert_eq!(colors["success.background"], "#5af78eff");
    assert_eq!(colors["danger.background"], "#ff5c57ff");
    assert_eq!(colors["base.green"], "#5af78eff");
    assert_eq!(colors["caret"], "#57c7ffff");
    assert_eq!(colors["selection.background"], "#57c7ff3d");
    assert!(colors.get("tab.background").is_none());

    let highlight = &dark["highlight"];
    assert_eq!(highlight["editor.background"], "#1e1f29ff");
    assert_eq!(highlight["editor.line_number"], "#5c5d5fff");
    assert_eq!(highlight["error"], "#ff5c57ff");
    assert!(highlight.get("background").is_none());
    let syntax = &highlight["syntax"];
    assert_eq!(syntax["keyword"], json!({ "color": "#ff6ac1ff" }));
    assert_eq!(
        syntax["comment"],
        json!({ "color": "#616263ff", "font_style": "italic" })
    );
    assert_eq!(
        syntax["title"],
        json!({ "color": "#57c7ffff", "font_weight": 700 })
    );
    assert_eq!(syntax["string.special.path"]["color"], "#f3f99dff");
    assert!(syntax.get("unknown_hint").is_none());
}

#[test]
fn taken_names_get_a_zed_suffix() {
    let mut value = family();
    value["themes"][1]["name"] = json!("Snazzy Dark");
    let (converted, set) = convert_value(&value, &["Snazzy Dark"]);
    let names: Vec<&str> = converted.themes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["Snazzy Dark (Zed)", "Snazzy Dark (Zed 2)"]);
    assert_eq!(set["themes"][0]["name"], "Snazzy Dark (Zed)");
}

#[test]
fn unnamed_themes_take_the_family_name() {
    let value = json!({
        "themes": [{ "appearance": "light", "style": {} }]
    });
    let (converted, set) = convert_value(&value, &[]);
    assert_eq!(converted.family, "Zed theme");
    assert_eq!(converted.file_stem, "zed-zed-theme");
    assert_eq!(set["themes"][0]["name"], "Zed theme");
    assert!(set.get("author").is_none());
}

#[rstest]
#[case::ascii("One Dark Pro", "zed-one-dark-pro")]
#[case::punctuation("  Snazzy -- Theme!! ", "zed-snazzy-theme")]
#[case::cyrillic("Тёмная тема", "zed-тёмная-тема")]
#[case::only_symbols("***", "zed-theme")]
fn file_stems_keep_letters_of_any_script(#[case] family: &str, #[case] stem: &str) {
    let value = json!({ "name": family, "themes": [{ "name": "x", "style": {} }] });
    let (converted, _) = convert_value(&value, &[]);
    assert_eq!(converted.file_stem, stem);
}

#[test]
fn comments_and_trailing_commas_are_accepted() {
    let source = r##"{
        // a user theme
        "name": "Mine", /* inline */
        "themes": [
            {
                "name": "Mine // not a comment",
                "appearance": "dark",
                "style": { "text": "#ffffff", "syntax": { "string": { "color": "#00ff00", }, }, },
            },
        ],
    }"##;
    let converted = convert(source, |_| false).unwrap();
    assert_eq!(converted.themes[0].name, "Mine // not a comment");
    let set: Value = serde_json::from_str(&converted.json).unwrap();
    assert_eq!(
        set["themes"][0]["highlight"]["syntax"]["string"]["color"],
        "#00ff00ff"
    );
}

#[rstest]
#[case::not_json("{ nope", "not valid JSON")]
#[case::no_themes(r#"{ "name": "x" }"#, "no \"themes\" list")]
#[case::empty_list(r#"{ "name": "x", "themes": [] }"#, "defines no themes")]
#[case::no_styles(r#"{ "themes": [{ "name": "a" }] }"#, "defines no themes")]
fn rejects_files_that_are_not_zed_themes(#[case] source: &str, #[case] message: &str) {
    let err = convert(source, |_| false).unwrap_err();
    assert!(err.to_string().contains(message), "{err}");
}

#[rstest]
#[case::short("#abc", Some("#aabbccff"))]
#[case::short_alpha("#abcd", Some("#aabbccdd"))]
#[case::rgb("#ABCDEF", Some("#abcdefff"))]
#[case::rgba("#11223344", Some("#11223344"))]
#[case::padded(" #112233 ", Some("#112233ff"))]
#[case::named("red", None)]
#[case::odd_length("#12345", None)]
#[case::not_hex("#gggggg", None)]
fn colors_normalize_to_rgba_hex(#[case] input: &str, #[case] expected: Option<&str>) {
    assert_eq!(color(&json!(input)).as_deref(), expected);
}

#[test]
fn null_and_non_string_colors_are_dropped() {
    assert_eq!(color(&Value::Null), None);
    assert_eq!(color(&json!(42)), None);
}

#[rstest]
#[case::bold(700.0, Some(700))]
#[case::rounds_up(450.0, Some(500))]
#[case::rounds_down(420.0, Some(400))]
#[case::clamps_high(1000.0, Some(900))]
#[case::clamps_low(0.0, Some(100))]
#[case::not_finite(f64::NAN, None)]
fn font_weights_snap_to_hundreds(#[case] weight: f64, #[case] expected: Option<u16>) {
    assert_eq!(font_weight(weight), expected);
}

#[rstest]
#[case::normal("normal", Some("normal"))]
#[case::italic("italic", Some("italic"))]
#[case::oblique("oblique", Some("italic"))]
#[case::underline("underline", Some("underline"))]
#[case::unknown("bold", None)]
fn font_styles_map_to_gpui_component(#[case] style: &str, #[case] expected: Option<&str>) {
    assert_eq!(font_style(style), expected);
}
