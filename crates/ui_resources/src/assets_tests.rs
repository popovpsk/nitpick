use gpui::AssetSource as _;

use super::Assets;

#[test]
fn an_empty_path_is_nothing_to_draw_and_a_missing_one_is_an_error() {
    assert!(matches!(Assets.load(""), Ok(None)));
    assert!(Assets.load("icons/no-such-icon.svg").is_err());
    assert!(matches!(Assets.load("icons/square.svg"), Ok(Some(_))));
    assert!(matches!(
        Assets.load("icons/arrow-down-up.svg"),
        Ok(Some(_))
    ));
}
