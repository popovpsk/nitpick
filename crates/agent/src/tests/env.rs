use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::process::{Environment, candidates, extensions_for, is_scrubbed, known_dirs};

fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect()
}

fn path_of(entries: &[&str]) -> String {
    std::env::join_paths(entries)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn shell(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[rstest]
#[case::nested_marker("CLAUDECODE", true)]
#[case::session_id("CLAUDE_CODE_SESSION_ID", true)]
#[case::child_session("CLAUDE_CODE_CHILD_SESSION", true)]
#[case::attended("CLAUDE_CODE_SESSION_ATTENDED", true)]
#[case::socket("CLAUDE_CODE_MESSAGING_SOCKET", true)]
#[case::token("CLAUDE_CODE_MESSAGING_TOKEN", true)]
#[case::execpath("CLAUDE_CODE_EXECPATH", true)]
#[case::sse_port("CLAUDE_CODE_SSE_PORT", true)]
#[case::pid("CLAUDE_PID", true)]
#[case::effort("CLAUDE_EFFORT", true)]
#[case::sdk("CLAUDE_AGENT_SDK_VERSION", true)]
#[case::orca("ORCA_TERMINAL_HANDLE", true)]
#[case::git_dir("GIT_DIR", true)]
#[case::git_work_tree("GIT_WORK_TREE", true)]
#[case::git_index("GIT_INDEX_FILE", true)]
#[case::provider_key("ANTHROPIC_API_KEY", false)]
#[case::other_provider_key("DEEPSEEK_API_KEY", false)]
#[case::config_dir("CLAUDE_CONFIG_DIR", false)]
#[case::git_identity("GIT_AUTHOR_NAME", false)]
#[case::path("PATH", false)]
fn knows_which_variables_to_scrub(#[case] name: &str, #[case] scrubbed: bool) {
    assert_eq!(is_scrubbed(OsStr::new(name)), scrubbed);
}

#[test]
fn removes_scrubbed_variables_from_the_process_environment_and_the_shell() {
    let env = Environment::merge(
        vars(&[
            ("CLAUDECODE", "1"),
            ("ORCA_X", "1"),
            ("GIT_DIR", "/x"),
            ("ANTHROPIC_API_KEY", "key"),
            ("PATH", "/usr/bin"),
        ]),
        &shell(&[
            ("CLAUDE_PID", "9"),
            ("GIT_WORK_TREE", "/y"),
            ("OPENAI_API_KEY", "k2"),
        ]),
        None,
    );
    for name in [
        "CLAUDECODE",
        "ORCA_X",
        "GIT_DIR",
        "CLAUDE_PID",
        "GIT_WORK_TREE",
    ] {
        assert!(env.get(name).is_none(), "{name} should be gone");
    }
    assert_eq!(env.get("ANTHROPIC_API_KEY"), Some(OsStr::new("key")));
    assert_eq!(env.get("OPENAI_API_KEY"), Some(OsStr::new("k2")));
}

#[test]
fn puts_the_shell_path_first_then_the_process_path_then_known_directories() {
    let home = Path::new("/home/user");
    let env = Environment::merge(
        vars(&[("PATH", &path_of(&["/usr/bin", "/bin"]))]),
        &shell(&[("PATH", &path_of(&["/opt/tools/bin", "/usr/bin"]))]),
        Some(home),
    );
    let mut expected: Vec<PathBuf> = ["/opt/tools/bin", "/usr/bin", "/bin"]
        .iter()
        .map(PathBuf::from)
        .collect();
    expected.extend(known_dirs(Some(home)));
    assert_eq!(
        std::env::split_paths(env.path()).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(env.get("PATH"), Some(env.path()));
}

#[test]
fn lists_the_known_directories() {
    let dirs = known_dirs(Some(Path::new("/h")));
    for expected in [
        "/h/.local/bin",
        "/h/.claude/local",
        "/h/.opencode/bin",
        "/h/.codex/bin",
        "/h/.npm-global/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ] {
        assert!(dirs.contains(&PathBuf::from(expected)), "{expected}");
    }
    assert_eq!(known_dirs(None).len(), 2);
}

#[test]
fn shell_variables_fill_gaps_without_overriding_the_process() {
    let env = Environment::merge(
        vars(&[("LANG", "C"), ("PATH", "/bin")]),
        &shell(&[
            ("LANG", "en_US.UTF-8"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
            ("DEEPSEEK_API_KEY", "secret"),
            ("PWD", "/somewhere"),
            ("OLDPWD", "/before"),
            ("SHLVL", "3"),
            ("_", "/usr/bin/env"),
        ]),
        None,
    );
    assert_eq!(env.get("LANG"), Some(OsStr::new("C")));
    assert_eq!(env.get("SSH_AUTH_SOCK"), Some(OsStr::new("/tmp/agent")));
    assert_eq!(env.get("DEEPSEEK_API_KEY"), Some(OsStr::new("secret")));
    for name in ["PWD", "OLDPWD", "SHLVL", "_"] {
        assert!(env.get(name).is_none(), "{name} should not be copied");
    }
}

#[test]
fn extra_variables_replace_existing_ones() {
    let mut env = Environment::merge(vars(&[("A", "1"), ("PATH", "/bin")]), &[], None);
    env.set("A", "2");
    env.set("B", "3");
    assert_eq!(env.get("A"), Some(OsStr::new("2")));
    assert_eq!(env.get("B"), Some(OsStr::new("3")));
    assert_eq!(env.vars().iter().filter(|(key, _)| key == "A").count(), 1);
}

#[rstest]
#[case::bare_name("claude", ".COM;.EXE;.CMD", &[".COM", ".EXE", ".CMD"])]
#[case::spaces_and_empties("claude", " .EXE ;; .CMD", &[".EXE", ".CMD"])]
#[case::explicit_extension("claude.cmd", ".COM;.EXE", &[""])]
#[case::no_pathext("claude", "", &[""])]
fn lists_the_extensions_to_try(
    #[case] program: &str,
    #[case] pathext: &str,
    #[case] expected: &[&str],
) {
    assert_eq!(extensions_for(program, pathext), expected);
}

#[test]
fn builds_one_candidate_per_extension() {
    let extensions = vec![".EXE".to_string(), ".CMD".to_string()];
    assert_eq!(
        candidates(Path::new("/d"), "tool", &extensions),
        vec![PathBuf::from("/d/tool.EXE"), PathBuf::from("/d/tool.CMD")]
    );
}
