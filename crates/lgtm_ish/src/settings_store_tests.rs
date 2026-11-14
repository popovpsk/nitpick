use rstest::rstest;

use super::{Settings, SyntaxPalettes, WrapLines, fit_split_ratio};

#[test]
fn settings_without_syntax_colors_still_load() {
    let settings: Settings =
        serde_json::from_str(r#"{ "theme": "system", "diff_layout": "unified" }"#).unwrap();
    assert_eq!(settings.syntax_colors, SyntaxPalettes::default());
    assert!(!settings.show_blame);
    assert!(!settings.show_whole_file);
    assert_eq!(settings.wrap_lines, WrapLines::SplitOnly);
    assert!(settings.editor_command.is_empty());
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

#[test]
fn settings_without_split_ratio_split_in_the_middle() {
    let settings: Settings = serde_json::from_str(r#"{ "tree_width": 300.0 }"#).unwrap();
    assert_eq!(settings.split_ratio, 0.5);
}

#[rstest]
#[case::kept(0.3, 1000., 0.3)]
#[case::left_column_minimum(0.05, 1000., 0.16)]
#[case::right_column_minimum(0.95, 1000., 0.84)]
#[case::component_range_on_a_wide_area(0.02, 4000., 0.1)]
#[case::too_narrow_for_two_minimums(0.3, 300., 0.5)]
#[case::unmeasured(0.3, 0., 0.5)]
#[case::not_a_number(f32::NAN, 1000., 0.5)]
fn split_ratio_keeps_both_columns_usable(
    #[case] ratio: f32,
    #[case] width: f32,
    #[case] expected: f32,
) {
    assert!((fit_split_ratio(ratio, width) - expected).abs() < 1e-6);
}
