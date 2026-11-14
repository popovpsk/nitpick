# slopcop

The application binary: startup (logging, secrets, settings, themes, language, updater), the
app menu, one window per repository, settings, and the glue that runs `git_repo` on GPUI's
background executor and `gitlab` / network git on `net_runtime`. Folder modules hold the
window (`workspace/`) and the review itself (`review/`); this file covers the crate-root files.

Not here: git (`git_repo`), the GitLab client (`gitlab`), the tree / viewed / line-map model
(`review_core`), token sealing (`secret_store`), icons and strings (`ui_resources`),
packaging (`Cargo.toml` `[package.metadata.packager]`, `packaging/`).

## Files

- `main.rs` — `main`, `build_window_options` (with the macOS `tabbing_identifier`),
  `open_window`, restoring the windows open at the last quit, app-level action handlers.
- `app_menu.rs` — menu bar actions and `set_menus` (rebuilt on a language change).
- `settings_store.rs` — `Settings` (serde, `settings.json` in the config dir, the GitLab
  token sealed by `secret_store`), the `SettingsStore` global, `update_settings`,
  `apply_theme`.
- `settings_view.rs` — the one-page settings screen: theme (hover previews), language,
  review options, GitLab URL + token with a connection check.
- `localization.rs` — resolves the saved language and exposes `tr()`.
- `recent.rs` — `RecentRepos` global (`recent.json` in the data dir): recent repositories
  with the last review of each, and the repositories of the windows open at quit.
- `window_registry.rs` — which repository each open window shows, persisted into
  `recent.json` as windows open, switch or close.
- `workspace/`, `review/` — see their `doc.md`.

## Rules

- Every window goes through `Root::new`: it mounts the notification, dialog and sheet layers
  and the Linux client-side border.
- Window options: `tabbing_identifier` on every window, so on macOS a new window becomes a
  native tab whenever the user's "Prefer tabs" setting asks for it; transparent,
  client-decorated windows on Linux; `app_owns_titlebar_drag` everywhere.
- The bundled themes register asynchronously; the saved theme is applied once at startup and
  again from `register_bundled_themes`' callback, because the registry is empty before it.
- The windows list in `recent.json` is not rewritten while the app quits, so closing all
  windows through Quit restores them all next launch; closing windows one by one does not.
- The `self-update` feature only forwards to `updater/self-update`; the portable Windows build
  turns it off and this crate has no `#[cfg]` for it.
- Quit on last window closed everywhere except macOS, where the app stays in the dock and
  `on_reopen` opens a new window.

## Decisions

- One repository per window; switching repositories happens in the title bar, a new
  repository opens a new window (a native tab on macOS).
- Settings are one screen inside the window, not a separate window.
