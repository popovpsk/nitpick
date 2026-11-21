# workspace

The root view of a window: the title bar, the macOS window-tab strip, and the screen below
it — welcome (no repository), start (choose a review) or the review.

Not here: the review itself (`review/doc.md`), settings storage (`settings_store.rs`),
window options and opening windows (`main.rs`), the settings window (`settings_window.rs`).

## Files

- `mod.rs` — `Workspace`: opening a repository (and its last review), switching screens,
  window title, the folder prompts (open one, add all from a folder), the repository
  picker's entries, window-level actions and bindings. Replacing the review, switching the
  repository or closing the window (`CloseWindow` and the window's own close, through
  `on_window_should_close`) asks first while the review holds an unsent comment
  (`ReviewView::has_unsent_comment`, `confirm_discard_comment`); Quit asks in the first
  window that holds one (`ask_before_quit` over the `OpenWorkspaces` global, deferred, since
  an app-level action runs while the dispatching window is borrowed).
- `title_bar.rs` — the repository picker trigger and its footer (open, add from folder), the
  review button, the "Restart to Update" button once the updater has an update staged
  (`updater::ready_note`; the workspace observes the updater), the settings button.
- `window_tabs.rs` — the strip of native window tabs (title centred, the close button at the
  tab's right edge on hover, as in Zed; its tooltip names `CloseWindow`'s key on the active tab
  only, the new-tab button shows `NewWindow`'s), dragging a tab onto another to reorder
  (`DraggedWindowTab`), and next/previous tab.
- `start.rs` — `StartView`: target/source branch pickers (fuzzy `Picker`s) for a
  comparison, the uncommitted-changes review, and the open merge requests (GitLab) or pull
  requests (GitHub) of the repository's remote on a configured host (list, filter, URL,
  `!123` or `#123`); the list reloads when a forge URL or token changes (`forge_key`, a hash,
  so other settings changes don't refetch), and by hand: Refresh beside the filter, Retry
  beside a load error.

## Spec

- Settings open through calls and events (`StartEvent::OpenSettings`,
  `ReviewEvent::OpenSettings`, `ReviewEvent::EditHiddenFiles`), never only through a
  dispatched action: an action starts from the focused element, and after a popover or menu
  closes nothing in the window may be focused, so it would not reach `Workspace`. Each passes
  this window and its repository as the `Opener`, and a repository switch tells the settings
  window. Every screen switch ends in `focus_screen`: the review's tree (its diff when the
  tree is hidden), or the window root, so menu items and shortcuts keep reaching the window.
  As a safety net, a render that finds nothing focused at all (no dialog or sheet open,
  the window active) focuses the current screen on the next frame. A focused element that
  is merely scrolled out of a virtualized list (a comment being typed) keeps its focus.
- A repository that fails to open (a folder that no longer exists) reports it through a
  deferred notification: `Workspace::new` runs inside `open_window` before the window has its
  `Root`, which holds the notification layer, so an immediate push panics.
- In fullscreen the title bar drops gpui-component's 80px macOS traffic-light inset (the
  lights are hidden there), so the repository picker starts at the edge.
- Repository picker: a fuzzy `Picker` over every known repository (name, then the path with
  `~` for the home dir; both are matched, the name preferred), in `recent.json` order, the
  current one selected (no checkmark). Enter or a click switches this window to the
  repository; the ↗ button on the row (as in Zed's recent projects), ⌘/Ctrl-Enter or
  ⌘/Ctrl-click opens it in a new window (a native tab on macOS when tabs are on). "Open Repository…" opens the
  folder in this window when it has none, in a new window otherwise. Each switch records the
  repository in `recent.json` and the window in `window_registry`. The entries are rebuilt
  whenever `RecentRepos` changes (any window), not on open.
- "Add Repositories from Folder…" (welcome screen, picker footer, File menu): the folder is
  scanned with `git_repo::find_repositories` on the background executor, every new path is
  added to `recent.json` without opening it, and a notification gives the count.
- Buttons that run a keymap command name its key in their tooltip (`tooltip_with_action`,
  context `Workspace`): the welcome and start screens' open, add and settings buttons as well as
  the title bar's.
- The review button shows only the review's title; viewed progress lives in the review
  header.
- Leaving the start or review screen closes its pickers: a popover's state lives in element
  state, which a hidden screen loses, so a picker left open would pop up again on return.
- Window tabs: gpui hides AppKit's own tab bar, so the strip is drawn here from
  `SystemWindowTabController` and only when the window has more than one tab. The tab title
  is the window title (`repo — review`), kept in sync with `update_tab_title`. Reordering
  (drag a tab onto another, as in Zed) moves it in the controller only
  (`update_tab_position`): AppKit's hidden tab bar keeps its own order; next/previous tab and
  the strip both use the controller's, AppKit's only picks the tab shown after the active one
  closes.
- The start screen preselects the remote default branch as the target and the checked-out
  branch as the source, and keeps the previous picks across a refresh. Opening a branch
  review from anywhere (including the review header's branch pickers) moves the start
  screen's picks to it. The merge-base explanation is the Compare button's tooltip.
- The request list picks the host from the remotes (`review::model::forge_remote` over
  `Settings::forge_hosts`: the GitLab URL when set, the GitHub URL or github.com): a remote
  on a connected host (URL and token) wins, else any matching remote, so a github.com
  `origin` does not hide a connected GitLab remote; within that, the `upstream` remote when
  it is on the same host as `origin` (a fork's `origin` has none of the project's requests),
  else `origin`, else the first. The project path comes from the remote URL. No match says that no remote
  points to a configured server when any host is connected, else that GitLab is not
  connected; a match on a
  host without a token says that host is not connected. The card title and the empty-list
  text follow the host ("merge requests" / "pull requests"). The input takes a GitLab or
  GitHub request URL (`forge::parse_request_url`, the host from the URL) or `!123` / `#123` /
  `123` for the listed host. Branch and request loads each keep one task, so a refresh
  cancels the previous load instead of racing it. Rows (author avatar, title, subtitle with
  the host's reference, author, branches and how long ago it was updated; the reviewers' and
  assignees' avatars, up to four, names in the tooltip; the comment count on GitLab;
  a lowercase filter key that includes those people) are built once per load, not per frame.
  Commit authors are not shown: they would cost a request per row.
- Closing a window tab: the current window closes through the click's own `window`, since
  `WindowHandle::update` on the window being dispatched fails.
