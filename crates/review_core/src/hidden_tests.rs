use rstest::rstest;

use crate::hidden::{HideRules, dir_name_pattern, dir_pattern, family_pattern, file_pattern};

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
fn family_patterns(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(family_pattern(path).as_deref(), expected);
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
