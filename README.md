# sloptaster

Tasting the slop so you don't have to merge it blind.

A side-by-side diff reviewer for huge vibe-coded merge requests, with per-file "viewed" progress.

Native desktop app for macOS, Windows and Linux, written in Rust on [GPUI](https://www.gpui.rs/) (`gpui-pre` + `gpui-component`).

## Build

```bash
cargo run --release            # default member is crates/sloptaster
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
| `crates/sloptaster` | The app binary: startup, menu, main window, packaging metadata |
| `crates/updater` | Auto-update from GitHub Releases (dmg / NSIS / AppImage) |
| `crates/ui_resources` | Embedded icons, app icon sources, localized strings |
| `crates/diagnostics` | Rolling log file and panic capture behind the `log` facade |
| `crates/net_runtime` | Shared tokio runtime for blocking network work |

## Releases

`.github/workflows/release.yml` (manual dispatch with a version) builds and publishes a draft release:

- macOS arm64 `.dmg`
- Windows x64 / arm64 NSIS installers and portable zips (portable builds have the updater compiled out)
- Linux x86_64 / arm64 AppImage, `.deb` and pacman archive

Publishing the release triggers `aur.yml` (`sloptaster-bin`) and `flatpak.yml` (signed repo on GitHub Pages).

Repository secrets: `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` (optional macOS self-signed identity, see `scripts/macos/create-signing-cert.sh`), `AUR_SSH_PRIVATE_KEY` (optional), `FLATPAK_GPG_PRIVATE_KEY` (required by the Flatpak workflow).

Maintenance scripts:

- `scripts/gen-icons.sh` — regenerate all app icons from `crates/ui_resources/assets/sloptaster.svg`
- `scripts/gen_flatpak_cargo_sources.py` — regenerate `packaging/flatpak/cargo-sources.json` after any `Cargo.lock` change (CI checks it)

## License

MIT
