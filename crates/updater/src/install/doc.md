# updater::install

Per-platform backends behind `download_and_stage`: download the asset (on macOS and Linux
also install it) and return a `Staged`, which `lib.rs` applies (`../doc.md`, "Behavior").

Not here: when to check, which asset, toasts (`../doc.md`); the feature-off stand-ins
(`../disabled/doc.md`).

## Files

- `mod.rs` — `Staged` (carries the installer path on Windows only), async dispatch by OS,
  `finalize_on_quit` (runs the installer on Windows, no-op elsewhere), the shared async
  `download_file`.
- `macos.rs`, `windows.rs`, `linux.rs` — one backend each.
- `tests.rs` — a download from a `wiremock` server to disk with its digest; the digest rule
  table.

## Download

- `download_file` streams the body chunk by chunk to the file (`tokio::fs`), never holding the
  whole asset in memory. It retries (5 attempts, 3 s apart) and resumes with `Range`; any
  reply but `206` restarts from zero, and so does a failed disk write (with `tokio::fs` a
  write error surfaces one write late, so the byte count can no longer be trusted). The
  file is flushed before a retry reopens it. The partial file is removed only once every
  attempt is spent. gzip is off so `Range` offsets and the digest are over the stored bytes.
- The body is stream-hashed (SHA-256) and compared with the asset's `digest`. A mismatch
  re-downloads from scratch (also how a bad resume heals). Enforced only when the API gave a
  `sha256:` digest, otherwise it warns and proceeds. It covers bytes altered in transit or on
  the CDN; it is not a substitute for signing.
- Timeouts: each attempt is capped at 15 min as a whole (per-request `timeout`); the stall
  detector is `http_client`'s 60 s read timeout, re-armed on every read. Do not shorten the
  15 min as if it were a headers timeout: it fails slow downloads of a healthy file.

## macOS

- The new bundle is rsynced from the mounted dmg over the running one (`app_path()`) during
  the download, so it is already new at `Ready` and the running process carries on. The bundle
  must be writable by the user. Only a `.app` is replaced (`can_install`): a bare binary's
  `app_path()` is its folder (`cargo run` gives `target/release`), which `rsync --delete`
  must never touch, so such a copy skips the update.
- `relaunch` is a detached `bash` that waits for our pid to go, then for `lsappinfo find
  pid=…` to come back empty (LaunchServices has let go; capped at 5 s), then `open`s the
  bundle. `apply_and_restart` calls it, then `cx.quit()`.
- Not `cx.restart()`: gpui's script waits only for the pid. LaunchServices learns of the exit
  later, and an `open` in that gap resolves to the old, dead instance and launches nothing
  (`-600 procNotFound`); the gap grows under load. Waiting on the LaunchServices view is what
  Sparkle does.
- Not `open -n`: `crates/nitpick/Info.plist` sets `LSMultipleInstancesProhibited` (macOS's
  single-instance guard), and with it LaunchServices ignores `-n` and picks the stale
  instance.

## Windows

- The `-setup.exe` goes to `cache_dir/nitpick/updates` (a running `.exe` cannot be overwritten)
  and runs once, from the quit handler: detached and windowless, `/S`, plus a `start` of the
  exe after an explicit apply.
- Unverified: that `/S` is silent for the cargo-packager NSIS installer
  (`installer-mode = "currentUser"`), that it handles the running instance, and whether it
  relaunches itself (making the `start` shim redundant). Check against a generated installer
  before relying on any of it.

## Linux

- The AppImage is downloaded as `<name>.new` beside `$APPIMAGE`, set to 0755 and renamed
  over it. A permission error becomes "update Nitpick the same way it was installed".
