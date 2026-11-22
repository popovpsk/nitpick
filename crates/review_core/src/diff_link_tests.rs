use rstest::rstest;

use crate::diff_link::{extract, format, parse};
use crate::line_map::line_maps;
use crate::*;

fn link(path: &str, range: Option<(Side, u32, u32)>) -> DiffLink {
    DiffLink {
        path: path.to_string(),
        range,
    }
}

#[rstest]
#[case::path_only("diff:src/main.rs", link("src/main.rs", None))]
#[case::single_new("diff:src/main.rs#R120", link("src/main.rs", Some((Side::New, 120, 120))))]
#[case::single_old("diff:src/main.rs#L7", link("src/main.rs", Some((Side::Old, 7, 7))))]
#[case::range_new("diff:a.rs#R120-R135", link("a.rs", Some((Side::New, 120, 135))))]
#[case::range_old("diff:a.rs#L1-L2", link("a.rs", Some((Side::Old, 1, 2))))]
#[case::reversed_ends("diff:a.rs#R135-R120", link("a.rs", Some((Side::New, 120, 135))))]
#[case::second_letter_omitted("diff:a.rs#R120-135", link("a.rs", Some((Side::New, 120, 135))))]
#[case::second_letter_omitted_old("diff:a.rs#L9-3", link("a.rs", Some((Side::Old, 3, 9))))]
#[case::same_ends("diff:a.rs#R5-R5", link("a.rs", Some((Side::New, 5, 5))))]
#[case::encoded_space("diff:my%20file.rs#R1", link("my file.rs", Some((Side::New, 1, 1))))]
#[case::encoded_parens("diff:a%28b%29.rs", link("a(b).rs", None))]
#[case::encoded_hash_and_percent("diff:a%23b%2525.rs", link("a#b%25.rs", None))]
#[case::lowercase_hex("diff:caf%c3%a9.rs", link("café.rs", None))]
#[case::raw_non_ascii("diff:src/файл.rs#R2", link("src/файл.rs", Some((Side::New, 2, 2))))]
#[case::upper_case_scheme("DIFF:a.rs", link("a.rs", None))]
fn parses(#[case] input: &str, #[case] expected: DiffLink) {
    assert_eq!(parse(input), Some(expected));
}

#[rstest]
#[case::not_diff("https://example.com/a.rs")]
#[case::empty("")]
#[case::scheme_only("diff:")]
#[case::empty_path_with_range("diff:#R1")]
#[case::line_zero("diff:a.rs#R0")]
#[case::range_with_zero("diff:a.rs#R0-R5")]
#[case::range_end_zero("diff:a.rs#R5-R0")]
#[case::mixed_sides("diff:a.rs#L1-R5")]
#[case::mixed_sides_new_old("diff:a.rs#R1-L5")]
#[case::no_side("diff:a.rs#120")]
#[case::lowercase_side("diff:a.rs#r120")]
#[case::empty_fragment("diff:a.rs#")]
#[case::missing_number("diff:a.rs#R")]
#[case::missing_second_number("diff:a.rs#R1-")]
#[case::missing_second_number_letter("diff:a.rs#R1-R")]
#[case::double_dash("diff:a.rs#R1--R5")]
#[case::three_parts("diff:a.rs#R1-R2-R3")]
#[case::signed_number("diff:a.rs#R+5")]
#[case::negative_number("diff:a.rs#R-5")]
#[case::overflow("diff:a.rs#R99999999999")]
#[case::trailing_garbage("diff:a.rs#R1x")]
#[case::second_fragment("diff:a.rs#R1#R2")]
#[case::truncated_escape("diff:a%2.rs")]
#[case::bad_escape("diff:a%zz.rs")]
#[case::invalid_utf8("diff:a%ff.rs")]
#[case::multibyte_scheme_boundary("diff日本")]
fn rejects(#[case] input: &str) {
    assert_eq!(parse(input), None);
}

#[rstest]
#[case::path_only(link("src/main.rs", None), "diff:src/main.rs")]
#[case::single(link("a.rs", Some((Side::New, 120, 120))), "diff:a.rs#R120")]
#[case::range(link("a.rs", Some((Side::New, 120, 135))), "diff:a.rs#R120-R135")]
#[case::old_range(link("a.rs", Some((Side::Old, 3, 4))), "diff:a.rs#L3-L4")]
#[case::space(link("my file.rs", None), "diff:my%20file.rs")]
#[case::parens(link("a(b).rs", None), "diff:a%28b%29.rs")]
#[case::hash_and_percent(link("a#b%c.rs", None), "diff:a%23b%25c.rs")]
#[case::non_ascii(link("café.rs", None), "diff:caf%C3%A9.rs")]
fn formats(#[case] input: DiffLink, #[case] expected: &str) {
    assert_eq!(format(&input), expected);
}

#[rstest]
#[case::plain("src/lib.rs")]
#[case::spaces("docs/my notes.md")]
#[case::parens("src/(group)/page (copy).tsx")]
#[case::brackets("app/[slug]/page.tsx")]
#[case::hash_percent("odd#name%20.txt")]
#[case::non_ascii("src/файл ü.rs")]
#[case::emoji("assets/🎉.png")]
#[case::markdown_breakers("a`b*c_d<e>f\"g'h\\i.rs")]
#[case::colon_and_at("pkg/@scope:name/x.js")]
#[case::query_like("a?b=c&d.rs")]
fn round_trips(
    #[case] path: &str,
    #[values(None, Some((Side::New, 1, 1)), Some((Side::Old, 10, 20)))] range: Option<(
        Side,
        u32,
        u32,
    )>,
) {
    let original = link(path, range);
    let text = format(&original);
    assert!(
        text.bytes().all(|b| b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b'%' | b':' | b'#')),
        "{text}"
    );
    assert_eq!(parse(&text), Some(original));
}

fn paths(links: &[DiffLink]) -> Vec<String> {
    links.iter().map(format).collect()
}

#[rstest]
#[case::single("See [the code](diff:a.rs#R1-R2).", &["diff:a.rs#R1-R2"])]
#[case::none("nothing here", &[])]
#[case::interleaved_http(
    "[a](https://x.io/a) then [b](diff:b.rs#R4) and [c](http://y) [d](diff:d.rs)",
    &["diff:b.rs#R4", "diff:d.rs"]
)]
#[case::list(
    "1. [one](diff:a.rs#R1)\n2. [two](diff:b.rs#L2)\n- [three](diff:c.rs)\n",
    &["diff:a.rs#R1", "diff:b.rs#L2", "diff:c.rs"]
)]
#[case::inline_text("before [x](diff:a.rs#R9) middle [y](diff:a.rs#R10) end", &["diff:a.rs#R9", "diff:a.rs#R10"])]
#[case::encoded_parens("[f](diff:a%28b%29.rs#R3)", &["diff:a%28b%29.rs#R3"])]
#[case::raw_balanced_parens("[f](diff:a(b).rs#R3) tail)", &["diff:a%28b%29.rs#R3"])]
#[case::angle_destination("[f](<diff:my file.rs#R3>)", &["diff:my%20file.rs#R3"])]
#[case::with_title("[f](diff:a.rs#R3 \"title\")", &["diff:a.rs#R3"])]
#[case::repeats_kept(
    "[a](diff:a.rs#R1) [b](diff:b.rs#R1) [a again](diff:a.rs#R1)",
    &["diff:a.rs#R1", "diff:b.rs#R1", "diff:a.rs#R1"]
)]
#[case::bare_text_is_not_a_link("see diff:a.rs#R1 and <diff:b.rs>", &[])]
#[case::label_text_is_not_a_link("[diff:a.rs#R1](https://example.com)", &[])]
#[case::not_a_link_target("(diff:a.rs#R1) and [x] (diff:b.rs)", &[])]
#[case::invalid_dropped("[a](diff:a.rs#R0) [b](diff:b.rs#R2) [c](diff:)", &["diff:b.rs#R2"])]
#[case::fenced_code_skipped(
    "[a](diff:a.rs#R1)\n```md\n[b](diff:b.rs#R1)\n```\n[c](diff:c.rs#R1)\n",
    &["diff:a.rs#R1", "diff:c.rs#R1"]
)]
#[case::tilde_fence_skipped("~~~\n[b](diff:b.rs)\n~~~\n[c](diff:c.rs)", &["diff:c.rs"])]
#[case::inline_code_skipped("`[b](diff:b.rs)` [c](diff:c.rs)", &["diff:c.rs"])]
#[case::unclosed_backtick_is_text("a ` b [c](diff:c.rs)", &["diff:c.rs"])]
#[case::escaped_bracket("\\[a](diff:a.rs) [b](diff:b.rs)", &["diff:b.rs"])]
#[case::non_ascii_around("Смотри [файл](diff:src/файл.rs#R2) — ü", &["diff:src/%D1%84%D0%B0%D0%B9%D0%BB.rs#R2"])]
#[case::second_letter_omitted("[a](diff:a.rs#R5-9)", &["diff:a.rs#R5-R9"])]
#[case::brackets_in_label("[a [b] c](diff:a.rs#R1)", &["diff:a.rs#R1"])]
#[case::image_like("![pic](https://x/y.png) [a](diff:a.rs)", &["diff:a.rs"])]
#[case::crlf("[a](diff:a.rs#R1)\r\n[b](diff:b.rs#R2)\r\n", &["diff:a.rs#R1", "diff:b.rs#R2"])]
fn extracts(#[case] markdown: &str, #[case] expected: &[&str]) {
    assert_eq!(paths(&extract(markdown)), expected);
}

const PATCH: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -10,3 +10,3 @@
 keep
-old
+new
 tail
";

#[rstest]
#[case::new_added(Some((Side::New, 11, 11)), Some(LineKind::Added { new: 11 }))]
#[case::old_removed(Some((Side::Old, 11, 11)), Some(LineKind::Removed { old: 11 }))]
#[case::context(Some((Side::New, 10, 10)), Some(LineKind::Context { old: 10, new: 10 }))]
#[case::outside(Some((Side::New, 50, 50)), None)]
#[case::range_starting_before_the_hunk(Some((Side::New, 1, 11)), Some(LineKind::Context { old: 10, new: 10 }))]
#[case::range_entirely_outside(Some((Side::New, 1, 5)), None)]
#[case::huge_range_is_bounded(Some((Side::New, 5000, u32::MAX)), None)]
#[case::no_range(None, None)]
fn locates(#[case] range: Option<(Side, u32, u32)>, #[case] expected: Option<LineKind>) {
    let maps = line_maps(PATCH);
    assert_eq!(maps.len(), 1);
    assert_eq!(link("a.rs", range).locate(&maps[0]), expected);
}
