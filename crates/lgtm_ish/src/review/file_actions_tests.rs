use git_repo::Remote;
use review_core::{Change, LineKind};
use rstest::rstest;

use super::{
    Forge, HostOs, Invocation, ProjectWeb, blob_url, change_line, editor_invocation, pick_line,
    project_web_from_mr, project_web_from_remotes,
};

fn remote(name: &str, url: &str) -> Remote {
    Remote {
        name: name.to_string(),
        url: url.to_string(),
    }
}

fn gitlab(base: &str) -> ProjectWeb {
    ProjectWeb {
        base: base.to_string(),
        forge: Forge::GitLab,
    }
}

#[rstest]
#[case::current(
    "https://gitlab.example.com/group/sub/project/-/merge_requests/42",
    Some("https://gitlab.example.com/group/sub/project")
)]
#[case::trailing_slash(
    "https://gitlab.example.com/group/project/-/merge_requests/7/",
    Some("https://gitlab.example.com/group/project")
)]
#[case::legacy(
    "https://gitlab.example.com/group/project/merge_requests/7",
    Some("https://gitlab.example.com/group/project")
)]
#[case::port(
    "http://localhost:8080/group/project/-/merge_requests/1",
    Some("http://localhost:8080/group/project")
)]
#[case::not_an_mr("https://gitlab.example.com/group/project", None)]
#[case::no_project("https://gitlab.example.com/-/merge_requests/1", None)]
fn project_web_from_an_mr_url(#[case] url: &str, #[case] expected: Option<&str>) {
    assert_eq!(project_web_from_mr(url), expected.map(gitlab));
}

#[rstest]
#[case::origin_first(
    vec![remote("upstream", "git@gitlab.com:up/project.git"), remote("origin", "git@gitlab.com:me/project.git")],
    Some(("https://gitlab.com/me/project", Forge::GitLab))
)]
#[case::first_without_origin(
    vec![remote("fork", "https://gitlab.example.com/team/app.git"), remote("other", "git@x.org:a/b.git")],
    Some(("https://gitlab.example.com/team/app", Forge::GitLab))
)]
#[case::github_ssh(
    vec![remote("origin", "git@github.com:owner/repo.git")],
    Some(("https://github.com/owner/repo", Forge::GitHub))
)]
#[case::github_https_with_user(
    vec![remote("origin", "https://user@GitHub.com/owner/repo")],
    Some(("https://GitHub.com/owner/repo", Forge::GitHub))
)]
#[case::ssh_scheme_with_port(
    vec![remote("origin", "ssh://git@gitlab.example.com:2222/group/sub/app.git")],
    Some(("https://gitlab.example.com/group/sub/app", Forge::GitLab))
)]
#[case::local_path(vec![remote("origin", "/srv/repos/app.git")], None)]
#[case::none(vec![], None)]
fn project_web_from_the_remotes(
    #[case] remotes: Vec<Remote>,
    #[case] expected: Option<(&str, Forge)>,
) {
    let expected = expected.map(|(base, forge)| ProjectWeb {
        base: base.to_string(),
        forge,
    });
    assert_eq!(project_web_from_remotes(&remotes), expected);
}

#[rstest]
#[case::gitlab(
    Forge::GitLab,
    "src/main.rs",
    Some(12),
    "https://h/g/p/-/blob/abc/src/main.rs#L12"
)]
#[case::github(
    Forge::GitHub,
    "src/main.rs",
    Some(3),
    "https://h/g/p/blob/abc/src/main.rs#L3"
)]
#[case::no_line(Forge::GitLab, "README.md", None, "https://h/g/p/-/blob/abc/README.md")]
#[case::escaped(
    Forge::GitLab,
    "docs/a b#1?.md",
    None,
    "https://h/g/p/-/blob/abc/docs/a%20b%231%3F.md"
)]
#[case::unicode(Forge::GitHub, "ü.txt", None, "https://h/g/p/blob/abc/%C3%BC.txt")]
fn blob_urls(
    #[case] forge: Forge,
    #[case] path: &str,
    #[case] line: Option<u32>,
    #[case] expected: &str,
) {
    let web = ProjectWeb {
        base: "https://h/g/p".to_string(),
        forge,
    };
    assert_eq!(blob_url(&web, "abc", path, line), expected);
}

fn change(first: LineKind, anchor: LineKind) -> Change {
    Change { first, anchor }
}

#[rstest]
#[case::added(change(LineKind::Added { new: 10 }, LineKind::Context { old: 7, new: 8 }), 10)]
#[case::removal_after_context(
    change(LineKind::Removed { old: 10 }, LineKind::Context { old: 8, new: 10 }),
    12
)]
#[case::removal_at_the_top(change(LineKind::Removed { old: 1 }, LineKind::Removed { old: 1 }), 1)]
#[case::added_first_line(change(LineKind::Added { new: 1 }, LineKind::Added { new: 1 }), 1)]
fn line_of_a_change(#[case] change: Change, #[case] expected: u32) {
    assert_eq!(change_line(&change), expected);
}

#[rstest]
#[case::cursor_wins(Some(20), false, Some(5), 20)]
#[case::selection_after_scroll(Some(20), true, Some(5), 5)]
#[case::cursor_after_scroll_without_selection(Some(20), true, None, 20)]
#[case::selection_without_cursor(None, false, Some(5), 5)]
#[case::first_change(None, false, None, 3)]
fn picked_line(
    #[case] cursor: Option<u32>,
    #[case] scrolled: bool,
    #[case] selection: Option<u32>,
    #[case] expected: u32,
) {
    let cursor = cursor.map(|new| change(LineKind::Added { new }, LineKind::Added { new }));
    let first = change(LineKind::Added { new: 3 }, LineKind::Added { new: 3 });
    assert_eq!(
        pick_line(cursor.as_ref(), scrolled, selection, Some(&first)),
        expected
    );
}

#[test]
fn no_change_picks_the_first_line() {
    assert_eq!(pick_line(None, false, None, None), 1);
}

fn shell(script: &str, shell: &str, file: &str, line: &str) -> Invocation {
    Invocation {
        program: shell.to_string(),
        args: vec!["-lc".to_string(), script.to_string()],
        env: vec![
            ("LGTM_ISH_FILE", file.to_string()),
            ("LGTM_ISH_LINE", line.to_string()),
            ("LGTM_ISH_ROOT", "/r".to_string()),
        ],
        check_exit: true,
    }
}

fn plain(program: &str, args: &[&str]) -> Invocation {
    Invocation {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: Vec::new(),
        check_exit: true,
    }
}

#[rstest]
#[case::default_mac("", HostOs::Mac, None, plain("open", &["/r/a b.rs"]))]
#[case::default_linux("  ", HostOs::Linux, None, plain("xdg-open", &["/r/a b.rs"]))]
#[case::default_windows("", HostOs::Windows, None, Invocation { check_exit: false, ..plain("explorer.exe", &["/r/a b.rs"]) })]
#[case::vscode(
    "code -g {file}:{line}",
    HostOs::Mac,
    Some("/bin/zsh"),
    shell(
        "code -g \"$LGTM_ISH_FILE\":\"$LGTM_ISH_LINE\"",
        "/bin/zsh",
        "/r/a b.rs",
        "7"
    )
)]
#[case::idea(
    "idea --line {line} {file}",
    HostOs::Linux,
    Some("/usr/bin/fish"),
    shell(
        "idea --line \"$LGTM_ISH_LINE\" \"$LGTM_ISH_FILE\"",
        "/usr/bin/fish",
        "/r/a b.rs",
        "7"
    )
)]
#[case::already_quoted(
    "subl \"{file}:{line}\"",
    HostOs::Mac,
    Some("/bin/bash"),
    shell(
        "subl \"$LGTM_ISH_FILE:$LGTM_ISH_LINE\"",
        "/bin/bash",
        "/r/a b.rs",
        "7"
    )
)]
#[case::no_file_placeholder(
    "zed",
    HostOs::Mac,
    Some("/bin/zsh"),
    shell("zed \"$LGTM_ISH_FILE\"", "/bin/zsh", "/r/a b.rs", "7")
)]
#[case::unknown_shell(
    "zed {file}",
    HostOs::Linux,
    Some("/usr/bin/nu"),
    shell("zed \"$LGTM_ISH_FILE\"", "/bin/sh", "/r/a b.rs", "7")
)]
#[case::no_shell(
    "zed {file}",
    HostOs::Mac,
    None,
    shell("zed \"$LGTM_ISH_FILE\"", "/bin/sh", "/r/a b.rs", "7")
)]
#[case::braces_kept(
    "ed {x} {file}",
    HostOs::Mac,
    Some("/bin/sh"),
    shell("ed {x} \"$LGTM_ISH_FILE\"", "/bin/sh", "/r/a b.rs", "7")
)]
#[case::windows_split(
    "code -g {file}:{line}",
    HostOs::Windows,
    None,
    plain("code", &["-g", "/r/a b.rs:7"])
)]
#[case::windows_quoted_program(
    "\"C:\\Program Files\\Sublime Text\\subl.exe\" \"{file}:{line}\"",
    HostOs::Windows,
    None,
    plain("C:\\Program Files\\Sublime Text\\subl.exe", &["/r/a b.rs:7"])
)]
#[case::single_quoted(
    "ed '{file}'",
    HostOs::Linux,
    Some("/bin/sh"),
    shell("ed ''\"$LGTM_ISH_FILE\"''", "/bin/sh", "/r/a b.rs", "7")
)]
#[case::windows_no_file("notepad++", HostOs::Windows, None, plain("notepad++", &["/r/a b.rs"]))]
#[case::zed_project(
    "zed {root} {file}:{line}",
    HostOs::Mac,
    Some("/bin/zsh"),
    shell(
        "zed \"$LGTM_ISH_ROOT\" \"$LGTM_ISH_FILE\":\"$LGTM_ISH_LINE\"",
        "/bin/zsh",
        "/r/a b.rs",
        "7"
    )
)]
#[case::windows_project(
    "goland {root} --line {line} {file}",
    HostOs::Windows,
    None,
    plain("goland", &["/r", "--line", "7", "/r/a b.rs"])
)]
fn editor_invocations(
    #[case] template: &str,
    #[case] os: HostOs,
    #[case] login: Option<&str>,
    #[case] expected: Invocation,
) {
    assert_eq!(
        editor_invocation(template, "/r", "/r/a b.rs", 7, os, login),
        expected
    );
}

#[rstest]
#[case::shell("code -g {file}:{line}", HostOs::Mac, "code -g /r/a b.rs:7")]
#[case::default("", HostOs::Linux, "xdg-open /r/a b.rs")]
#[case::project(
    "open -na \"GoLand.app\" --args {root} --line {line} {file}",
    HostOs::Mac,
    "open -na \"GoLand.app\" --args /r --line 7 /r/a b.rs"
)]
#[case::windows(
    "idea --line {line} {file}",
    HostOs::Windows,
    "idea --line 7 /r/a b.rs"
)]
fn invocations_display_with_the_values(
    #[case] template: &str,
    #[case] os: HostOs,
    #[case] expected: &str,
) {
    let invocation = editor_invocation(template, "/r", "/r/a b.rs", 7, os, Some("/bin/zsh"));
    assert_eq!(invocation.display(), expected);
}

#[test]
fn a_file_name_with_placeholders_is_not_substituted_again() {
    let invocation = editor_invocation(
        "ed {file} {line}",
        "C:\\",
        "C:\\{line}.rs",
        3,
        HostOs::Windows,
        None,
    );
    assert_eq!(
        invocation.args,
        vec!["C:\\{line}.rs".to_string(), "3".to_string()]
    );
}

#[cfg(unix)]
#[rstest]
#[case::bare("printf '%s|' {file}:{line}", "{f}:42|")]
#[case::double_quoted("printf '%s|' \"{file}:{line}\" \"x {line}\"", "{f}:42|x 42|")]
#[case::single_quoted("printf '%s|' '{file}' 'at {line}'", "{f}|at 42|")]
#[case::escaped_quote("printf '%s|' \\\" {file}", "\"|{f}|")]
#[case::project("printf '%s|' {root} {file}:{line}", "/r|{f}:42|")]
fn the_shell_receives_the_file_and_line_unmangled(#[case] template: &str, #[case] expected: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("out.txt");
    let file = dir.path().join("it's a \"file\" $HOME *.rs");
    let template = format!("{template} > '{}'", out.display());
    let mut invocation = editor_invocation(
        &template,
        "/r",
        &file.to_string_lossy(),
        42,
        HostOs::Linux,
        None,
    );
    invocation
        .env
        .push(("HOME", dir.path().to_string_lossy().into_owned()));
    let mut child = super::spawn(&invocation, dir.path()).unwrap();
    assert!(child.wait().unwrap().success());
    let written = std::fs::read_to_string(&out).unwrap();
    assert_eq!(written, expected.replace("{f}", &file.to_string_lossy()));
}

#[test]
fn programs_are_found_with_their_extension_on_the_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(second.join("code.CMD"), "").unwrap();
    std::fs::write(second.join("code.EXE"), "").unwrap();
    let path = std::env::join_paths([&first, &second]).unwrap();
    let exts = ".COM;.EXE;.BAT;.CMD";
    assert_eq!(
        super::find_program("code", &path, exts),
        Some(second.join("code.EXE"))
    );
    assert_eq!(super::find_program("code.cmd", &path, exts), None);
    assert_eq!(super::find_program("bin/code", &path, exts), None);
    assert_eq!(super::find_program("missing", &path, exts), None);
}

#[rstest]
#[case::mac(
    HostOs::Mac,
    "open -na \"GoLand.app\" --args {root} --line {line} {file}"
)]
#[case::linux(HostOs::Linux, "goland {root} --line {line} {file}")]
#[case::windows(HostOs::Windows, "goland {root} --line {line} {file}")]
fn presets_open_the_repository_as_a_project(#[case] os: HostOs, #[case] goland: &str) {
    let presets = super::editor_presets(os);
    assert!(
        presets
            .iter()
            .all(|(_, template)| template.contains("{root}")
                && template.contains("{file}")
                && template.contains("{line}"))
    );
    let find = |name: &str| {
        presets
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| t.as_str())
    };
    assert_eq!(find("GoLand"), Some(goland));
    assert_eq!(find("VS Code"), Some("code {root} -g {file}:{line}"));
    assert_eq!(find("Zed"), Some("zed {root} {file}:{line}"));
}
