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
- `recent.rs` — `RecentRepos` global (`recent.json` in the data dir): every known
  repository (opened or added from a folder) with the last review of each, and the
  repositories of the windows open at quit.
- `recent_tests.rs` — ordering, bulk add, and the stored format on a `TempDir`.
- `picker.rs` — `Picker`: the fuzzy picker every repository and branch chooser uses, a
  `Popover` around gpui-component's searchable `List` with a `PickerDelegate`;
  `PickerEntry`, `branch_entries`, `PickerEvent` (`Open`, `Confirm { entry, secondary }`).
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
- `recent.json` keeps every known repository, unbounded, in display order: opened ones by
  `opened_at` newest first, then never-opened ones (`opened_at` 0, from "Add Repositories
  from Folder…") by folder name, case-insensitively. Opening sets `opened_at` to
  `max(now, newest + 1)`, so opens within one second still order. The format is the old
  one (`repos`, `open_windows`, same fields), so older files load unchanged.
- `AddRepositories` and `OpenRepository` with no window open just open a welcome window.
- Picker: the owner keeps one `Picker` entity per chooser and renders
  `picker.popover(id, trigger, cx)`; the open state lives in the `Picker` (the popover is
  controlled), so the owner observes it to re-render. Entries are replaced with
  `set_entries` when the data changes, never built in `render`; matching runs on the
  background executor with `review_core::rank`, and match ranges are precomputed per row.
  Rows are drawn from the entry list they were ranked against, and a confirm is dropped
  while newer entries are still being ranked, so an index in `Confirm` always refers to
  the entries the owner set last.
  With an empty query the checked entry (the current repository or branch) is selected,
  otherwise the best match. Enter or click confirms; ⌘/Ctrl-Enter or ⌘/Ctrl-click confirms
  with `secondary`; Escape or a click outside closes.

## Decisions

- One repository per window; switching repositories happens in the title bar, a new
  repository opens a new window (a native tab on macOS).
- Settings are one screen inside the window, not a separate window.
