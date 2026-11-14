# review

One review: a branch comparison, a GitLab merge request or the uncommitted changes of the work
tree, loaded into a file tree, a diff of one file at a time, per-file "viewed" marks, an
optional blame of the shown file and, for a merge request, its discussions.
`ReviewView` is the entity; the window creates one per opened review.

Not here: choosing what to review (`workspace/start.rs`), the git operations
(`git_repo`), the tree and viewed models (`review_core`), the GitLab API (`gitlab`).

## Files

- `mod.rs` — `ReviewView`: loading (`load_review`), installing results, viewed marks and their
  save, tree selection and collapse state, file navigation, pull, discussions and the comment
  composer; actions and key bindings.
- `model.rs` — `ReviewSource`, `LoadedReview`, `ReviewFile`, `build` (changed files + full
  context patch → parsed `DiffFile`s with `review_core::language_override` applied, line
  maps (plus the shown patch's map with "ignore whitespace"), fingerprints, tree),
  `gitlab_remote`, `mr_refspecs`, `comparison_for_mr`.
- `model_grammars.rs` — `register_highlight_queries`: highlight queries for grammars
  gpui-component 0.9.1 links without one (C#, Swift, CMake from their grammar crates; C++ as
  the C++ query plus C's; proto and GraphQL from `../../queries/*/highlights.scm`).
- `model_tests.rs` — the whole pipeline on a throwaway repository; every enabled language
  produces syntax colors.
- `viewed.rs` — `SharedViewed`: one `ViewedStore` entity per repository for the whole app,
  a debounced save, `flush_all` at quit.
- `tree_panel.rs` — the file tree panel (`uniform_list` of precomputed `TreeRow`s) and its
  keyboard handling.
- `diff_panel.rs` — the review header (branch pickers with pull buttons, the MR, or the work
  tree; viewed progress), the file toolbar and the `Diff` element; the sliding, resizable tree
  panel; `Render for ReviewView`.
- `branch_picker.rs` — `BranchPickers`: one fuzzy `Picker` per side of a branch review, the
  branch list loaded when one opens, and the pick → `ReviewEvent::Open`.
- `comments.rs` — discussion cards and the composer as `Diff` annotations, and the sheet with
  general and outdated discussions.
- `blame.rs` — `ToggleBlame`, `BlameState` (the cache, the shown marks, the fetch), the
  toolbar toggle (`render_blame_toggle`) and the chips drawn in the `Diff`'s marker lane
  (`BlameMarks::render_chip`).

## Spec

- What a branch review shows is `git_repo::Repo::compare`: the merge base of the source with
  the target and the target's upstream, so commits the target gained since never show up.
  A merge request shows GitLab's own `diff_refs` (`base_sha`..`head_sha`); when either commit
  is missing locally, `refs/merge-requests/<iid>/head` is fetched into `refs/lgtm-ish/mr/<iid>`
  (hidden from branch lists) from the remote whose URL path is the MR's project, else
  `origin`, else any remote on the GitLab host.
- The work-tree review (`ReviewSource::WorkingTree`) diffs `HEAD` (the empty tree in an
  unborn repository) against the files on disk: staged, unstaged and untracked
  (`git_repo::Repo::working_tree_*`). It has no commits, no pull and no comments. It reloads
  when its window becomes active, so edits made in an editor show up on return.
- The patch is requested with full-file context so the diff can expand folded lines; over
  48 MB it is recomputed with 3 lines and the header says so.
- Exactly one file is in the `DiffState` at a time; selecting a file replaces it. This bounds
  highlighting work on huge reviews and keeps the file header a stable scope.
- `LoadedReview::files` is in patch order; the tree's `file_order` is the navigation order.
  `line_maps` and the parsed files are index-aligned; if the counts ever differ the review
  still loads but diff comments are disabled (`comments_aligned`).
- Hidden files: the patterns from settings (all repositories, then this repository) hide
  matching files from the review's own `tree` (`FileTree::build_with`), so they are out of
  the tree, the progress, change and file navigation, auto-mark and the "first unviewed"
  pick; `LoadedReview::tree` keeps every file and is reused when nothing is hidden. The
  header's "N hidden" popover lists them with the pattern that hid each one, the part of the
  path the pattern matched highlighted (`HiddenFile::matched`, computed with the rules), and
  "Edit hidden files…" there or in the tree menu emits `ReviewEvent::OpenSettings`; a click
  shows that file without putting it back in the tree; from it, file and change navigation
  and `advance_on_viewed` continue with the nearest visible file in full tree order
  (`visible_neighbour`), and auto-mark never marks it. Changing the patterns re-applies
  them at once; if the shown file becomes hidden the next unviewed visible file opens, and
  when files reappear after everything was hidden one opens again. Tree selection survives
  a rebuild by node path, never by a row index that now points elsewhere. The
  tree's context menu adds a pattern (this file or folder in this repository, the file's
  family such as `*_mock.go` here or everywhere, a folder name everywhere).
- Viewed marks key on `path` + `old_blob..new_blob`. Every review of a repository, in any
  window, shares one store entity and observes it, so marks made in one window show in the
  others and no window overwrites another's marks with a stale copy. A change schedules one
  save 250 ms later (a newer change replaces the pending one, so snapshots land in order);
  quitting flushes synchronously. Marking the current file viewed jumps to the next unviewed
  file in tree order when `advance_on_viewed` is set; a folder toggles all files under it.
- A discussion is placed on a line only when its position's `head_sha` is the review's head;
  others are listed as outdated in the comments sheet with the general ones. A new line
  comment sends `old_line`/`new_line` from `LineMap::locate`: both for a context line, one
  for an added or removed line.
- Tree rows are rebuilt when data changes (load, viewed, collapse, discussions), never in
  `render`; the list closure only reads them.
- Default keys live in `keymap.rs` and can be changed in Settings; the contexts below are
  what the registry binds. Key bindings in the tree (context `ReviewTree`) are single letters (`j/k/h/l`, `space`/`v`,
  `n/p` for the next/previous file); anywhere in the review (context `Review`) they are
  `space` (viewed) and modifier or function keys (`alt-up/down` and `f7`/`shift-f7` for the
  next/previous change, `alt-left/right` for the previous/next file, `cmd-b`, `cmd-r`,
  `cmd-1/2`). `space` and the `alt-` arrows pass through (`cx.propagate`) while an input has
  focus, so typing a comment never marks, switches files or eats a word jump. No
  `alt-<letter>` binding: on macOS bindings match before text input, so it would eat the
  character the layout types. Toolbar tooltips name their key through `tooltip_with_action`,
  so they follow the bindings.
- Change navigation (the toolbar arrows, `step_change`) follows IntelliJ's next/previous
  difference with a change cursor in place of the caret. The changes are
  `ReviewFile::changes`, runs of changed lines from the displayed patch (so with "ignore
  whitespace" a whitespace-only edit is not a stop). A jump scrolls to the change's anchor,
  up to two context lines above it, never to a folded line, since `scroll_to_line` would
  unfold it. Next goes to the change after the cursor, so from a freshly opened file it is
  the first one. Previous goes to the one before it; after a manual scroll (wheel, scrollbar
  or a click inside the diff, or the diff's own scroll keys) it first returns to the cursor's
  change. Past the last (or before the first) change, one press shows a hint for 3 s and a
  second press opens the neighbouring file in tree order, at its first (or last) change; a
  manual scroll or another jump disarms it. No wrap-around after the last file. Opening a
  file any other way resets the cursor. The viewport position itself is private to
  gpui-component's `DiffState`, hence the cursor and the scroll detection.- Auto-mark (`auto_mark_viewed`): leaving a file for another marks it viewed if it was shown
  for at least `AUTO_MARK_DWELL` (1 s), so running through the tree with j/k marks nothing,
  and unless the user unmarked it by hand (`kept_unviewed`, keyed by path + fingerprint so it
  survives reloads of this review). Reloads and the first file of a load never mark anything.
- A reload that finds the same files with the same fingerprints (and the same whitespace and
  context setting) changes nothing, and a reload that keeps the shown file's fingerprint
  keeps its `DiffState`, so the work-tree reload on window activation does not reset the
  scroll, the expanded context or the selection. A merge request always reinstalls.
- The tree panel is drawn here rather than with `h_resizable`, so it can slide: toggling
  animates its width over `TREE_SLIDE` (ease-out, as in pawse) while the content keeps its
  full width inside an `overflow_hidden` box, and a closing panel stays rendered until the
  slide ends, covered so it takes no clicks. The first render of a review does not animate.
  Hiding the tree while it has focus moves focus to the diff first; otherwise the focused
  element vanishes and the `Review` key bindings (⌘B included) stop reaching the review. The drag handle on its inner
  edge resizes it within `TREE_WIDTH_RANGE`; the width is saved to settings 400 ms after the
  last move by a detached task, so closing the review right after a drag still saves it.
- The comment composer survives file switches and reloads; only Cancel or a successful post
  of that same composer closes it.
- Notifications go to the review's own window (`window_handle`) through `cx.defer`, never to
  whichever window happens to be active, and never inside that window's own update.
- The review observes the settings: a new diff layout applies at once, a change of "ignore
  whitespace" reloads.
- The `DiffState` mode is decided per file (`diff_mode`): an added or deleted file is always
  unified (one column, whatever the layout setting), anything else follows the setting. It
  is per file because exactly one file is in the state; the toolbar keeps showing the saved
  layout. `sync_diff_mode` applies it before a new file goes in (`show_file`) and when the
  layout changes (`apply_settings`, `set_layout`), never from `render`.
- With the Split setting the `Diff` soft-wraps, whole-file views included: the component
  splits its content width in half and wrapping pins that width to the viewport, so both
  sides are always exactly half. With Unified, long lines scroll horizontally, whole files
  too. Wrapping follows the setting rather than the mode shown, so a file looks the same as
  its neighbours under one setting, and the header's right-hand label stays in view. The
  file header's "Added file" / "Deleted file" label is gpui-component's own.
- Blame (`settings.show_blame`, the toolbar toggle and `ToggleBlame`, no default key; the
  setting is global, so every window follows it): the shown file is blamed off the UI thread
  at the review's head commit, or the work tree for the uncommitted-changes review, and a
  deleted file at `base_sha` on its old side. A fetch starts `BLAME_DELAY` (120 ms) after the
  file is shown, so running through files with j/k starts no `git blame` (a started one runs
  to its end: the blocking child is not killed when its task is dropped). Results are cached
  by (rev, path) for the installed review; a reload that installs a new review drops the
  cache and refetches, and an unchanged reload keeps it like everything else but retries
  failed blames. Chips are shown only for the file and content they were computed for: the
  same key with the same fingerprint keeps its chips until the refetch lands (a work-tree
  reload does not blink); anything else shows an empty lane until its result arrives, and a
  result that lands after the file changed is cached but not shown. A failed blame leaves
  the lane empty and shows one notification until the next reload or toggle.
- Blame chips use the `Diff`'s marker lane (between the line numbers and the code, one row
  high and one row wide, in each column in split mode, once in unified): no extra row. A
  chip sits on the first line of each run from one commit and on the first line of each
  block the default folding shows (`review_core::LineMap::shown_block_starts`); nothing on
  the other side. It is the author's initials on a tint picked from the commit id
  (`review_core::blame::tint_index`) over six theme hues at two strengths with the theme's
  foreground, so it reads in light and dark themes; uncommitted lines get a muted "you". The
  tooltip shows author, relative date (computed when it opens), short id and summary. While
  blame is on, the lane stays even before results arrive or when there are none (a
  placeholder marker on no line), so the code does not shift sideways on every file switch.
  Markers and chip data are built when a result lands; render only clones the marker list
  the component's builder takes and looks a chip up by binary search.
- With "ignore whitespace", line maps come from a second patch without `-w`, matched by path,
  so comment positions follow GitLab's real diff rather than the whitespace-blind one;
  `ReviewFile::display_map` keeps the shown patch's map (`shown_map`), from which `changes`
  and blame's block starts are taken.
- A discussion whose position has no line (file or image comments) goes to the sheet.
- Header branch chips: the name opens a fuzzy branch picker for that side only (the branch
  list is read with `git for-each-ref` each time a picker opens, off the UI thread);
  picking emits `ReviewEvent::Open` with that side replaced and the other kept, and the
  window opens it as a new review exactly like the start screen does, recording it as the
  repository's last review. Picking the other side's branch, or the current one, does
  nothing.
- The review header shows viewed progress before the refresh button (`viewed/total` and a
  thin bar, the count in the tooltip), read from `progress()`; the title bar does not.
- Highlight queries are registered once, by the first `parse`, through gpui-component's
  `LanguageRegistry::register` (same grammar, new query); every highlighter is built after
  that, since files reach the `DiffState` only from a parsed review. The proto and GraphQL
  queries are ours, written against those grammars' node types; capture names must be
  gpui-component highlight names (a dotted name falls back to its prefix).

## Decisions

- One file at a time rather than all files in one scrolling list.
- Viewed progress is local; GitLab has no API for its "viewed" checkboxes.
- The uncommitted-changes review compares with `HEAD`, not with a target branch.
- Pull is fast-forward only and only from the branch chips of a branch review.
- Added and deleted files show as one whole file, never as a split with an empty side.
- The split divider is fixed in the middle: gpui-component's `Diff` has no API for a column
  ratio, and a draggable divider would mean forking it.
- "Viewed" has one control, the toolbar button; the file header has none.
- Blame is compact: one initials chip per run of a commit in the marker lane, details in the
  tooltip; not Zed's per-line author and date column.
