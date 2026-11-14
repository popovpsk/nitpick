# lgtm-ish

Looks good to me… ish. For huge merge requests you shouldn't approve blind.

A side-by-side diff reviewer for huge vibe-coded merge requests, with per-file "viewed" progress.

- Shows only what a merge request brings, like GitLab: the diff from the merge base (also
  against the target's upstream), so commits the target branch gained since never leak in.
- File tree on the left or right with compressed folders, collapse/expand all, and "viewed"
  marks that roll up to folders and reset when a file changes again; optionally a file is
  marked viewed as soon as you move on from it.
- Review your own uncommitted changes (staged, unstaged and untracked) before you commit.
- Hide noise by gitignore-style patterns, for all repositories or one (`*_mock.go`,
  `vendor/`, `/gen/`): hidden files leave the tree and the progress, the header says how
  many; right-click a file or folder to hide its kind.
- Syntax highlighting (tree-sitter: Go, Rust, TypeScript/TSX, JavaScript, Python, Java, Kotlin,
  Scala, Swift, C, C++, C#, Ruby, PHP, Lua, Elixir, Zig, SQL, GraphQL, JSON, YAML, TOML,
  Markdown, Bash, Protobuf, CSS, HTML, Make, CMake, diff), split or unified layout, expandable
  context.
- Bundled themes, syntax colors from any of them chosen separately, and Zed theme import.
- One fast-forward-only "pull" button per branch to bring in fresh changes.
- Optional GitLab connection: open merge requests by list, URL or `!iid`, read every
  discussion on the lines it belongs to, comment, reply, resolve.
- One repository per window; switch from the title bar, or open more as macOS native tabs.
- Read-only: it never edits your files.

Native desktop app for macOS, Windows and Linux, written in Rust on [GPUI](https://www.gpui.rs/) (`gpui-pre` + `gpui-component`).

## Keys

| Where | Key | Action |
| --- | --- | --- |
| File tree | `↑`/`↓`, `j`/`k` | Move and open the file |
| File tree | `←`/`→`, `h`/`l` | Collapse / expand the folder |
| File tree | `space`, `v` | Toggle viewed (a folder toggles every file in it) |
| File tree | `n` / `p` | Next / previous file |
| Review | `space` | Toggle viewed for the current file |
| Review | `⌥↓` / `⌥↑`, `F7` / `⇧F7` | Next / previous change; past the last one, press again for the next file |
| Review | `⌥→` / `⌥←` | Next / previous file |
| Review | `⌘B` | Show / hide the file tree |
| Review | `⌘1` / `⌘2` | Focus the tree / the diff |
| Review | `⌘R` | Refresh (refetch the merge request and its comments) |
| Comment box | `⌘⏎` | Send |
| Window | `⌘O`, `⌘N`, `⌘,` | Open repository, new review, settings |

On Windows and Linux use `Ctrl` for `⌘`. Every shortcut can be changed in Settings → Shortcuts: click a key, press the new one. (The Ctrl variants of the ⌘ shortcuts are no longer bound on macOS.)

## Build

```bash
cargo run --release            # default member is crates/lgtm_ish
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Linux build dependencies (Debian/Ubuntu):

```bash
sudo apt-get install libfontconfig-dev libwayland-dev libxkbcommon-x11-dev build-essential cmake clang
```

## Layout

| Crate | What |
| --- | --- |
| `crates/lgtm_ish` | The app binary: windows, title bar, review screen, settings, packaging metadata |
| `crates/git_repo` | Everything that talks to `git` (CLI): merge-base comparisons, patches, pull, fetch |
| `crates/gitlab` | Async GitLab REST client: merge requests, discussions, comments |
| `crates/review_core` | File tree, viewed-progress store, diff line → GitLab position mapping |
| `crates/http_client` | Shared `reqwest` + rustls setup (OS certificate store) |
| `crates/secret_store` | Seals the GitLab token inside `settings.json` |
| `crates/updater` | Auto-update from GitHub Releases (dmg / NSIS / AppImage) |
| `crates/ui_resources` | Embedded icons, bundled themes, app icon sources, localized strings (en, ru) |
| `crates/diagnostics` | Rolling log file and panic capture behind the `log` facade |
| `crates/net_runtime` | The one tokio runtime: all network I/O runs on it |

## Releases

`.github/workflows/release.yml` (manual dispatch with a version) builds and publishes a draft release:

- macOS arm64 `.dmg`
- Windows x64 / arm64 NSIS installers and portable zips (portable builds have the updater compiled out)
- Linux x86_64 / arm64 AppImage, `.deb` and pacman archive

Publishing the release triggers `aur.yml` (`lgtm-ish-bin`) and `flatpak.yml` (signed repo on GitHub Pages).

Repository secrets: `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` (optional macOS self-signed identity, see `scripts/macos/create-signing-cert.sh`), `AUR_SSH_PRIVATE_KEY` (optional), `FLATPAK_GPG_PRIVATE_KEY` (required by the Flatpak workflow).

Maintenance scripts:

- `scripts/gen-icons.sh` — regenerate all app icons from `crates/ui_resources/assets/lgtm-ish.svg`
- `scripts/gen_flatpak_cargo_sources.py` — regenerate `packaging/flatpak/cargo-sources.json` after any `Cargo.lock` change (CI checks it)

## License

MIT
