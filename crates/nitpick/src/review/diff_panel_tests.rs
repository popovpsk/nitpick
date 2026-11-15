use gpui::px;
use rstest::rstest;

use super::ProgressFit;

#[rstest]
#[case::everything(220., 3, true, 56.)]
#[case::a_longer_count_needs_more_room(220., 7, false, 56.)]
#[case::count_goes_first(200., 3, false, 56.)]
#[case::then_the_bar_shrinks(170., 3, false, 38.)]
#[case::down_to_a_floor(100., 3, false, 12.)]
fn the_tree_header_drops_the_count_then_shrinks_the_bar(
    #[case] width: f32,
    #[case] label_chars: usize,
    #[case] label: bool,
    #[case] bar: f32,
) {
    let fit = ProgressFit::for_width(px(width), px(16.), label_chars);
    assert_eq!((fit.label, fit.bar), (label, px(bar)));
}
