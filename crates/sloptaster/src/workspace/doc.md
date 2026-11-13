# workspace

The root view of a window: the title bar, the macOS window-tab strip, and the screen below
it — welcome (no repository), start (choose a review), the review, or settings.

Not here: the review itself (`review/doc.md`), settings storage (`settings_store.rs`),
window options and opening windows (`main.rs`).

## Files

- `mod.rs` — `Workspace`: opening a repository (and its last review), switching screens,
  window title, the folder prompt, window-level actions and bindings.
- `title_bar.rs` — the repository picker (recent repositories, open another), the review
  button with progress, the settings button.
- `window_tabs.rs` — the strip of native window tabs, and next/previous tab.
- `start.rs` — `StartView`: target/source branch pickers for a comparison, and the GitLab
  merge requests of the repository's GitLab remote (list, filter, URL or `!iid`).

## Spec

- Repository picker: a click switches this window to the repository; ⌘/Ctrl-click opens it
  in a new window. "Open Repository…" opens the folder in this window when it has none, in a
  new window otherwise. Each switch records the repository in `recent.json` and the window in
  `window_registry`.
- Window tabs: gpui hides AppKit's own tab bar, so the strip is drawn here from
  `SystemWindowTabController` and only when the window has more than one tab. The tab title
  is the window title (`repo — review`), kept in sync with `update_tab_title`.
- The start screen preselects the remote default branch as the target and the checked-out
  branch as the source, and keeps the previous picks across a refresh.
- The merge request list needs a GitLab URL and token in settings and a remote whose host
  matches that URL; the project path comes from the remote URL.
