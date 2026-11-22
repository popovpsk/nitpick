use rstest::rstest;

use crate::process::{
    BEGIN_MARKER, END_MARKER, StderrTail, login_shell, parse_snapshot, split_words,
};

#[rstest]
#[case::single_word("claude", &["claude"])]
#[case::empty("", &[])]
#[case::only_spaces("   \t ", &[])]
#[case::arguments("my-claude-code --flag value", &["my-claude-code", "--flag", "value"])]
#[case::quoted_program(
    r#""C:\Program Files\tool\claude.exe" --a"#,
    &[r"C:\Program Files\tool\claude.exe", "--a"]
)]
#[case::quoted_argument(r#"a "b c" d"#, &["a", "b c", "d"])]
#[case::empty_quotes(r#"a "" b"#, &["a", "", "b"])]
#[case::quote_inside_word(r#"a="b c" d"#, &["a=b c", "d"])]
#[case::unterminated_quote(r#"a "b c"#, &["a", "b c"])]
#[case::extra_spaces("  npx   -y  @scope/tool acp ", &["npx", "-y", "@scope/tool", "acp"])]
fn splits_a_command_into_words(#[case] input: &str, #[case] expected: &[&str]) {
    assert_eq!(split_words(input), expected);
}

#[rstest]
#[case::zsh(Some("/bin/zsh"), "/bin/zsh")]
#[case::padded(Some(" /bin/bash "), "/bin/bash")]
#[case::fish(Some("/usr/local/bin/fish"), "/usr/local/bin/fish")]
#[case::relative(Some("zsh"), "/bin/sh")]
#[case::not_a_shell(Some("/usr/bin/evil"), "/bin/sh")]
#[case::empty(Some(""), "/bin/sh")]
#[case::unset(None, "/bin/sh")]
fn picks_a_login_shell(#[case] shell: Option<&str>, #[case] expected: &str) {
    assert_eq!(login_shell(shell), expected);
}

fn pairs(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    vars.iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn reads_the_environment_between_the_markers() {
    let text = format!(
        "welcome to the shell\n{BEGIN_MARKER}\nPATH=/a:/b\nLANG=en_US.UTF-8\nEMPTY=\nURL=http://x?a=b\n{END_MARKER}\ntrailing noise\n"
    );
    assert_eq!(
        parse_snapshot(&text),
        Some(pairs(&[
            ("PATH", "/a:/b"),
            ("LANG", "en_US.UTF-8"),
            ("EMPTY", ""),
            ("URL", "http://x?a=b"),
        ]))
    );
}

#[test]
fn joins_a_value_that_spans_lines() {
    let text = format!("{BEGIN_MARKER}\nFOO=first\nsecond line\nBAR=x\n{END_MARKER}\n");
    assert_eq!(
        parse_snapshot(&text),
        Some(pairs(&[("FOO", "first\nsecond line"), ("BAR", "x")]))
    );
}

#[rstest]
#[case::no_begin("PATH=/a\n__NITPICK_ENV_END__\n")]
#[case::no_end("__NITPICK_ENV_BEGIN__\nPATH=/a\n")]
#[case::empty("")]
fn rejects_an_incomplete_snapshot(#[case] text: &str) {
    assert_eq!(parse_snapshot(text), None);
}

#[test]
fn keeps_only_the_last_four_kilobytes_of_stderr() {
    let tail = StderrTail::default();
    for index in 0..1000 {
        tail.push_line(&format!("line number {index}"));
    }
    let text = tail.text();
    assert!(text.len() <= 4096);
    assert!(text.ends_with("line number 999"));
    assert!(!text.contains("line number 0\n"));
}

#[test]
fn cuts_stderr_on_a_character_boundary() {
    let tail = StderrTail::default();
    tail.push_line(&"é".repeat(3000));
    let text = tail.text();
    assert!(text.len() <= 4096);
    assert!(text.chars().all(|c| c == 'é'));
}

#[cfg(unix)]
#[tokio::test]
async fn the_login_shell_reports_a_path() {
    let vars = crate::process::capture_from("/bin/sh")
        .await
        .expect("the shell answered");
    assert!(vars.iter().any(|(key, _)| key == "PATH"));
}
