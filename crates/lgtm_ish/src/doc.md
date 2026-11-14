# lgtm-ish

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
  `apply_theme`; syntax colors: `SyntaxPalettes`, `apply_syntax_colors` and the `Theme`
  observer that keeps it applied, hover previews, `syntax_palette_names`; hidden-file
  patterns (`hidden_files` for every repository, `repo_hidden_files` keyed by the work
  tree's path), `HideScope`, `add_hide_pattern`.
- `settings_store_tests.rs` — `settings.json` without the newer fields still loads; syntax
  palettes per appearance.
- `settings_view.rs` — the one-page settings screen: theme and syntax colors (hover
  previews), Zed theme import, language, review options, hidden-file patterns (all
  repositories, and the window's repository when it has one; saved 400 ms after typing
  stops by a detached task, invalid lines named under the field), GitLab URL + token with a
  connection check. A save applies the difference between the list the field was loaded
  from and what was typed (`review_core::hidden::merge_patterns`), so a pattern added from a
  tree menu meanwhile, in any window, survives; a field that is not focused reloads when the
  settings change, and `set_repository` reloads both each time the screen opens.
- `localization.rs` — resolves the saved language and exposes `tr()`.
- `keymap.rs` — `COMMANDS`, the registry of every user-facing shortcut (id, group, key
  context, default keys with `secondary-` for ⌘/Ctrl, action, title); `init` and `apply`
  build the live keymap; `keys`, `conflict`, `assign`, `reset` for the settings screen.
- `keymap_tests.rs` — defaults parse on every OS and never repeat a key in one context.
- `shortcut_dialog.rs` — the dialog that records a new shortcut (`ShortcutRecorder`), and
  the keystroke chip helper the settings list shares with it.
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

- Shortcuts: modules no longer bind their own keys; `keymap::init` runs after every other
  `init`, snapshots what gpui-component and the fixed bindings registered (`BaseKeymap`),
  then `apply` clears the keymap and binds the snapshot plus `COMMANDS` with the user's
  overrides. It re-applies whenever `settings.keybindings` changes and resets the app menus,
  so tooltips (`tooltip_with_action`) and menu key equivalents follow at once. An override
  is the whole key list of one command (an empty list unbinds it); a list equal to the
  defaults is dropped, so stored files only carry real changes. Keys are stored in the
  host's spelling (`cmd-b` on macOS, `ctrl-b` elsewhere). Defaults bind ⌘ on macOS and Ctrl
  elsewhere only, not both.
- Recording a shortcut uses `intercept_keystrokes`, which runs before key bindings, so
  ⌘W or Space are recorded instead of acting; it listens only in the dialog's window and
  only while recording, stops on Escape (never recorded) or after one keystroke, and ignores
  bare modifiers. After a keystroke the interceptor stays until the dialog closes (`on_close`,
  Cancel, Save, Remove) and swallows auto-repeats of that key, so holding Enter records it
  without also confirming. Every command using the key in the same context, or where either
  is global, is named and loses the key on save; resetting a command takes its defaults back
  the same way, so no key ever ends up bound twice. Window commands refuse keys without ⌘,
  Ctrl or a function key, since they would fire while typing in any field; review handlers
  pass keys on (`cx.propagate`) while an input has focus, so a plain key there is safe. A key
  also bound by gpui-component itself (the diff's arrows, `tab`) is flagged, since the deeper
  built-in binding wins where that control has focus.

- Syntax colors replace only `highlight_theme.style.syntax`, keeping the app theme's other
  highlight fields. `Theme::change` (theme switch, theme files reloaded, startup) restores
  the theme's own palette, so a `Theme` global observer re-applies the chosen one; it writes
  only when the syntax differs, which also ends the loop its own write starts. A palette is
  used only while its appearance matches the app's (light and dark are chosen separately);
  a name the registry does not have yet (themes load asynchronously at startup) or no longer
  has falls back to the theme's own palette, and a theme without one to gpui-component's
  default palette for the appearance (the component itself would keep the previous one).
- Zed import runs the picker and the conversion off the UI thread and reports success or
  the error as a notification in the settings window. Zed's theme files are listed in the
  import menu, found once when the settings screen opens.
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
  Rows are drawn from the entry list they were ranked against. New entries under the same
  query keep the selected entry; a new query resets the selection. Enter before the
  background ranking lands ranks once on the UI thread, so `Confirm` always refers to the
  latest query and the entries the owner set last.
  With an empty query the checked entry (the current repository or branch) is selected,
  otherwise the best match. Enter or click confirms; ⌘/Ctrl-Enter or ⌘/Ctrl-click confirms
  with `secondary`; Escape or a click outside closes.
- Highlighting languages are gpui-component `tree-sitter-*` features in `Cargo.toml`. The SQL
  grammar (`tree-sitter-sequel`) requires `cc ~1.2.1`, so `Cargo.lock` keeps `cc` on 1.2.x;
  the grammar crates listed directly are there only for their highlight queries
  (`review/model_grammars.rs`) and must stay on the versions gpui-component resolves.

## Decisions

- One repository per window; switching repositories happens in the title bar, a new
  repository opens a new window (a native tab on macOS).
- Settings are one screen inside the window, not a separate window.
- Syntax colors default to the app theme's; the choices are the palettes of every registered
  theme of the current appearance, bundled or imported, one choice per appearance.
- Zed themes are imported as files (converted once), not read from Zed at startup.
