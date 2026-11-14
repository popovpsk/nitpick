use super::{Settings, SyntaxPalettes};

#[test]
fn settings_without_syntax_colors_still_load() {
    let settings: Settings =
        serde_json::from_str(r#"{ "theme": "system", "diff_layout": "unified" }"#).unwrap();
    assert_eq!(settings.syntax_colors, SyntaxPalettes::default());
    assert!(!settings.show_blame);
}

#[test]
fn syntax_palettes_are_kept_per_appearance() {
    let mut palettes = SyntaxPalettes::default();
    palettes.set(true, Some("Gruvbox Dark".into()));
    assert_eq!(palettes.get(true).map(String::as_str), Some("Gruvbox Dark"));
    assert_eq!(palettes.get(false), None);
    assert_eq!(
        serde_json::to_value(&palettes).unwrap(),
        serde_json::json!({ "dark": "Gruvbox Dark" })
    );
    let settings: Settings =
        serde_json::from_str(r#"{ "syntax_colors": { "dark": "Gruvbox Dark" } }"#).unwrap();
    assert_eq!(settings.syntax_colors, palettes);
    palettes.set(true, None);
    assert_eq!(
        serde_json::to_value(&palettes).unwrap(),
        serde_json::json!({})
    );
}
