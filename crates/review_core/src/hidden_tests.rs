use rstest::rstest;

use crate::hidden::{
    HideRules, dir_name_pattern, dir_pattern, family_pattern, file_pattern, matched_ranges,
    merge_patterns,
};

fn rules(global: &[&str], repository: &[&str]) -> HideRules {
    let own = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (rules, errors) = HideRules::new(&own(global), &own(repository));
    assert!(errors.is_empty(), "{errors:?}");
    rules
}

#[rstest]
#[case::suffix_anywhere("*_mock.go", "internal/store/user_mock.go", true)]
#[case::suffix_top_level("*_mock.go", "user_mock.go", true)]
#[case::suffix_other_file("*_mock.go", "internal/store/user.go", false)]
#[case::dir_anywhere("vendor/", "a/vendor/lib/x.go", true)]
#[case::dir_prefix_is_not_the_dir("vendor/", "vendored/x.go", false)]
#[case::anchored_dir("/gen/", "gen/api.pb.go", true)]
#[case::anchored_dir_elsewhere("/gen/", "pkg/gen/api.pb.go", false)]
#[case::anchored_file("/go.sum", "go.sum", true)]
#[case::anchored_file_nested("/go.sum", "tools/go.sum", false)]
#[case::double_star("**/testdata/**", "pkg/a/testdata/in.json", true)]
#[case::extension("*.pb.go", "api/v1/service.pb.go", true)]
#[case::star_dir_any_depth("*/mocks/*", "internal/store/mocks/user.go", true)]
#[case::star_dir_top_level("*/mocks/*", "store/mocks/user.go", true)]
#[case::star_dir_nested_file("*/mocks/*", "a/mocks/deep/user.go", true)]
#[case::star_dir_other_name("*/mocks/*", "a/mockserver/user.go", false)]
#[case::middle_slash_stays_anchored("src/gen/", "pkg/src/gen/a.go", false)]
fn patterns_follow_gitignore(#[case] pattern: &str, #[case] path: &str, #[case] hidden: bool) {
    assert_eq!(rules(&[pattern], &[]).is_hidden(path), hidden);
}

#[test]
fn repository_rules_add_to_and_can_unhide_global_ones() {
    let rules = rules(
        &["*_mock.go", "# a comment", ""],
        &["!keep_mock.go", "/docs/"],
    );
    assert!(rules.is_hidden("a/user_mock.go"));
    assert!(!rules.is_hidden("a/keep_mock.go"));
    assert!(rules.is_hidden("docs/readme.md"));
    assert_eq!(
        rules.matching_rule("x/user_mock.go").as_deref(),
        Some("*_mock.go")
    );
    assert_eq!(
        rules.matching_rule("docs/a/b.md").as_deref(),
        Some("/docs/")
    );
    assert_eq!(rules.matching_rule("src/main.go"), None);
}

#[test]
fn no_rules_hide_nothing_and_bad_rules_are_reported() {
    let (empty, errors) = HideRules::new(&[], &["   ".to_string()]);
    assert!(empty.is_empty() && errors.is_empty());
    assert!(!empty.is_hidden("anything.go"));

    let (rules, errors) = HideRules::new(&["src/{a,b".to_string(), "*.lock".to_string()], &[]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].pattern, "src/{a,b");
    assert!(rules.is_hidden("Cargo.lock"));
}

#[rstest]
#[case::mockery("internal/user_mock.go", Some("*_mock.go"))]
#[case::dashed("web/button-stories.tsx", Some("*-stories.tsx"))]
#[case::double_extension("api/service.pb.go", Some("*.pb.go"))]
#[case::test_file("src/app.test.ts", Some("*.test.ts"))]
#[case::plain("main.go", Some("*.go"))]
#[case::leading_underscore("_layout.tsx", Some("*.tsx"))]
#[case::no_extension("Makefile", None)]
#[case::dotfile(".gitignore", None)]
#[case::bracketed("pages/[id]_test.ts", Some("*_test.ts"))]
#[case::bracketed_ext("a.[x]", Some(r"*.\[x\]"))]
fn family_patterns(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(family_pattern(path).as_deref(), expected);
}

#[rstest]
#[case::next_route("app/blog/[slug]/page.tsx", "app/blog/[slug]", "app/blog/s/page.tsx")]
#[case::braces("tpl/{{name}}/a.txt", "tpl/{{name}}", "tpl/n/a.txt")]
#[case::comment_like("#notes/a.md", "#notes", "notes/a.md")]
#[case::negation_like("!keep/a.md", "!keep", "keep/a.md")]
#[case::star("a*b/c.go", "a*b", "axxb/c.go")]
fn menu_patterns_match_the_literal_name_only(
    #[case] file: &str,
    #[case] dir: &str,
    #[case] other: &str,
) {
    let exact = rules(&[], &[&file_pattern(file)]);
    assert!(exact.is_hidden(file));
    let folder = rules(&[], &[&dir_pattern(dir)]);
    assert!(folder.is_hidden(file));
    assert!(!folder.is_hidden(other));
    let named = rules(&[&dir_name_pattern(dir)], &[]);
    assert!(named.is_hidden(file));
    assert!(!named.is_hidden(other));
}

#[test]
fn rooted_or_odd_paths_never_match_and_never_panic() {
    let rules = rules(&["*.go"], &[]);
    assert!(rules.is_hidden("/a.go"));
    assert!(!rules.is_hidden(""));
    assert!(!rules.is_hidden("/"));
}

#[test]
fn exact_patterns_are_anchored() {
    assert_eq!(file_pattern("src/a.go"), "/src/a.go");
    assert_eq!(dir_pattern("src/gen"), "/src/gen/");
    assert_eq!(dir_name_pattern("pkg/mocks"), "mocks/");
    let rules = rules(&[], &[&dir_pattern("src/gen")]);
    assert!(rules.is_hidden("src/gen/a/b.go"));
    assert!(!rules.is_hidden("other/src/gen/b.go"));
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_edit_replaces_the_list_when_nothing_changed_meanwhile() {
    let mut list = owned(&["a", "b"]);
    merge_patterns(&mut list, &owned(&["a", "b"]), &owned(&["!b", "a"]));
    assert_eq!(list, owned(&["!b", "a"]));
}

#[test]
fn an_edit_keeps_patterns_added_elsewhere_meanwhile() {
    let mut list = owned(&["a", "b", "*_mock.go"]);
    merge_patterns(&mut list, &owned(&["a", "b"]), &owned(&["a", "c"]));
    assert_eq!(list, owned(&["a", "*_mock.go", "c"]));
}

#[rstest]
#[case::extension("*.json", "packaging/aur/sources.json", &[".json"])]
#[case::suffix("*_mock.go", "internal/store/user_mock.go", &["_mock.go"])]
#[case::dir_anywhere("vendor/", "a/vendor/lib/x.go", &["vendor"])]
#[case::dir_not_the_file("x.go/", "x.go/x.go", &["x.go"])]
#[case::anchored_dir("/gen/", "gen/api.pb.go", &["gen"])]
#[case::exact_file("/src/store.rs", "src/store.rs", &["src", "store.rs"])]
#[case::double_star("**/testdata/**", "pkg/a/testdata/in.json", &["testdata"])]
#[case::inner_slash("docs/*.md", "docs/readme.md", &["docs", ".md"])]
#[case::class("*.[jt]s", "web/app.ts", &[".", "s"])]
#[case::escaped("/app/blog/\\[slug\\]/", "app/blog/[slug]/page.tsx", &["app", "blog", "[slug]"])]
#[case::question("file?.txt", "a/file1.txt", &["file", ".txt"])]
#[case::unicode("*ё.md", "docs/всё.md", &["ё.md"])]
#[case::negation("!keep.go", "keep.go", &[])]
#[case::braces("*.{rs,go}", "src/main.go", &[".go"])]
#[case::nested_braces("{docs,{a,b}}/*.md", "b/x.md", &["b", ".md"])]
fn matched_ranges_cover_the_literal_part(
    #[case] pattern: &str,
    #[case] path: &str,
    #[case] expected: &[&str],
) {
    let parts: Vec<&str> = matched_ranges(pattern, path)
        .into_iter()
        .map(|range| &path[range])
        .collect();
    assert_eq!(parts, expected);
}

#[test]
fn many_stars_do_not_backtrack_exponentially() {
    let pattern = format!("{}*b", "*a".repeat(16));
    let path = format!("{}c/{}", "a".repeat(40), "a".repeat(60));
    let started = std::time::Instant::now();
    assert!(matched_ranges(&pattern, &path).is_empty());
    let deep = format!("{}x", "**/".repeat(12));
    let long: String = (0..60).map(|i| format!("d{i}/")).collect::<String>() + "y";
    assert!(matched_ranges(&deep, &long).is_empty());
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
}

#[test]
fn a_leading_star_dir_reports_and_highlights_what_was_typed() {
    let rules = rules(&["*/mocks/*"], &["!*/mocks/keep.go"]);
    let path = "internal/store/mocks/user.go";
    assert_eq!(rules.matching_rule(path).as_deref(), Some("*/mocks/*"));
    assert!(!rules.is_hidden("a/b/mocks/keep.go"));
    let ranges = matched_ranges("*/mocks/*", path);
    let lit: String = ranges.iter().map(|r| &path[r.clone()]).collect();
    assert_eq!(lit, "mocks");
}
