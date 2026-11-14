# workspace

The root view of a window: the title bar, the macOS window-tab strip, and the screen below
it — welcome (no repository), start (choose a review), the review, or settings.

Not here: the review itself (`review/doc.md`), settings storage (`settings_store.rs`),
window options and opening windows (`main.rs`).

## Files

- `mod.rs` — `Workspace`: opening a repository (and its last review), switching screens,
  window title, the folder prompts (open one, add all from a folder), the repository
  picker's entries, window-level actions and bindings.
- `title_bar.rs` — the repository picker trigger and its footer (open, add from folder), the
  review button, the settings button.
- `window_tabs.rs` — the strip of native window tabs, and next/previous tab.
- `start.rs` — `StartView`: target/source branch pickers (fuzzy `Picker`s) for a
  comparison, the uncommitted-changes review, and the GitLab merge requests of the
  repository's GitLab remote (list, filter, URL or `!iid`).

## Spec

- Settings open and close through calls and events (`StartEvent::OpenSettings`,
  `ReviewEvent::OpenSettings`, `SettingsEvent::Close`), never only through a dispatched
  action: an action starts from the focused element, and after a popover or menu closes
  nothing in the window may be focused, so it would not reach `Workspace`. Every screen
  switch ends in `focus_screen`: the settings screen, the review's tree (its diff when the
  tree is hidden), or the window root, so menu items and shortcuts keep reaching the window.
  As a safety net, a render that finds focus nowhere inside the window (and no dialog or
  sheet open, while the window is active) focuses the current screen on the next frame.
- In fullscreen the title bar drops gpui-component's 80px macOS traffic-light inset (the
  lights are hidden there), so the repository picker starts at the edge.
- Repository picker: a fuzzy `Picker` over every known repository (name, then the path with
  `~` for the home dir; both are matched, the name preferred), in `recent.json` order, the
  current one checked. Enter or a click switches this window to the repository;
  ⌘/Ctrl-Enter or ⌘/Ctrl-click opens it in a new window (a native tab on macOS when tabs are
  on). The trigger's tooltip says so; there is no hint line. "Open Repository…" opens the
  folder in this window when it has none, in a new window otherwise. Each switch records the
  repository in `recent.json` and the window in `window_registry`. The entries are rebuilt
  whenever `RecentRepos` changes (any window), not on open.
- "Add Repositories from Folder…" (welcome screen, picker footer, File menu): the folder is
  scanned with `git_repo::find_repositories` on the background executor, every new path is
  added to `recent.json` without opening it, and a notification gives the count.
- The review button shows only the review's title; viewed progress lives in the review
  header.
- Leaving the start or review screen closes its pickers: a popover's state lives in element
  state, which a hidden screen loses, so a picker left open would pop up again on return.
- Window tabs: gpui hides AppKit's own tab bar, so the strip is drawn here from
  `SystemWindowTabController` and only when the window has more than one tab. The tab title
  is the window title (`repo — review`), kept in sync with `update_tab_title`.
- The start screen preselects the remote default branch as the target and the checked-out
  branch as the source, and keeps the previous picks across a refresh. Opening a branch
  review from anywhere (including the review header's branch pickers) moves the start
  screen's picks to it. The merge-base explanation is the Compare button's tooltip.
- The merge request list needs a GitLab URL and token in settings and a remote whose host
  matches that URL; the project path comes from the remote URL. Branch and MR loads each keep
  one task, so a refresh cancels the previous load instead of racing it. MR rows (title,
  subtitle, lowercase filter key) are built once per load, not per frame.
- Closing a window tab: the current window closes through the click's own `window`, since
  `WindowHandle::update` on the window being dispatched fails.
