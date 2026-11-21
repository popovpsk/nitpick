use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use git_repo::Repo;
use gpui::{ImageFormat, SMOOTH_SVG_SCALE_FACTOR, SvgRenderer};

use super::{Loaded, Source, load};

const SCRUBBED_ENV: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

fn git(dir: &Path, args: &[&str], input: Option<&[u8]>) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in SCRUBBED_ENV {
        command.env_remove(name);
    }
    let mut child = command.spawn().expect("git runs");
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"], None);
    let repo = Repo::open(dir.path()).unwrap();
    (dir, repo)
}

fn blob(dir: &Path, bytes: &[u8]) -> Source {
    Source::Blob(git(dir, &["hash-object", "-w", "--stdin"], Some(bytes)))
}

fn renderer() -> SvgRenderer {
    SvgRenderer::new(Arc::new(()))
}

#[test]
fn a_plain_svg_still_renders_through_gpui() {
    let (dir, repo) = repo();
    let source = blob(
        dir.path(),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect width="8" height="4" fill="red"/></svg>"#,
    );
    let Ok(Loaded::Image(image, _)) = load(&repo, &source, ImageFormat::Svg, renderer()) else {
        panic!("a plain SVG renders");
    };
    let size = image.size(0);
    let scale = SMOOTH_SVG_SCALE_FACTOR as i32;
    assert_eq!((size.width.0, size.height.0), (8 * scale, 4 * scale));
}

#[test]
fn an_svg_naming_a_local_file_is_refused() {
    let (dir, repo) = repo();
    let picture = dir.path().join("private.png");
    image::RgbaImage::new(1, 1).save(&picture).unwrap();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="{}" width="8" height="8"/></svg>"#,
        picture.display()
    );
    let source = blob(dir.path(), svg.as_bytes());
    let error = load(&repo, &source, ImageFormat::Svg, renderer())
        .err()
        .expect("refused before gpui reads the file");
    assert!(error.to_string().contains("not embedded"), "{error}");
}

#[test]
fn a_work_tree_path_that_is_not_a_file_is_refused() {
    let (dir, repo) = repo();
    let folder = dir.path().join("looks-like.png");
    std::fs::create_dir(&folder).unwrap();
    let error = load(&repo, &Source::Disk(folder), ImageFormat::Png, renderer())
        .err()
        .expect("a folder is never read");
    assert!(error.to_string().contains("not a regular file"), "{error}");
}
