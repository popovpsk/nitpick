# review

One review: a branch comparison, a GitLab merge request or the uncommitted changes of the work
tree, loaded into a file tree, a diff of one file at a time, per-file "viewed" marks and, for a
merge request, its discussions.
`ReviewView` is the entity; the window creates one per opened review.

Not here: choosing what to review (`workspace/start.rs`), the git operations
(`git_repo`), the tree and viewed models (`review_core`), the GitLab API (`gitlab`).

## Files

- `mod.rs` — `ReviewView`: loading (`load_review`), installing results, viewed marks and their
  save, tree selection and collapse state, file navigation, pull, discussions and the comment
  composer; actions and key bindings.
- `model.rs` — `ReviewSource`, `LoadedReview`, `ReviewFile`, `build` (changed files + full
  context patch → parsed `DiffFile`s with `review_core::language_override` applied, line
  maps, fingerprints, tree), `gitlab_remote`, `mr_refspecs`, `comparison_for_mr`.
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
  tree; viewed progress), the file toolbar (change and file navigation, the shown file's
  label as a cached `FileLabel`, viewed and layout controls) and the `Diff` element with its
  movable split divider; the sliding, resizable tree panel; `Render for ReviewView`.
- `branch_picker.rs` — `BranchPickers`: one fuzzy `Picker` per side of a branch review, the
  branch list loaded when one opens, and the pick → `ReviewEvent::Open`.
- `comments.rs` — discussion cards and the composer as `Diff` annotations, and the sheet with
  general and outdated discussions.

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
  highlighting work on huge reviews, and is why the `Diff` draws no file header
  (`header_visible(false)`): the toolbar names the file instead. File-level annotations
  still render, since the component keeps them when the header is hidden.
- The file toolbar, left to right: tree toggle, previous/next change, previous/next file
  (`PreviousFile`/`NextFile`, disabled without a visible neighbour), `n / m` in tree order,
  then the file: status icon in the tree's colors, the path with the directory muted and the
  name in the foreground color, ellipsized from the start so the name stays, `+a`/`−d` when
  non-zero, and a "new file" / "deleted" / "renamed" tag (the renamed tag's tooltip names the
  old path); the cross-file hint follows it. Viewed and layout controls sit on the right.
  The whole path has one font weight and the name stands out by color only: GPUI measures
  truncation with the base font, so a bolder run would be clipped and a lighter one would
  leave a gap before the counts.
- `FileLabel` holds the toolbar's strings (path, counts, position, neighbours). It is rebuilt
  at the start of the toolbar render only when the review `Rc`, the tree `Arc` or the shown
  file changed, so a steady frame formats nothing; it keeps no translated text, so a
  language switch needs no rebuild.
- Added and deleted files (one column) are drawn without the change background
  (`change_background(false)`) and with `DiffChangeIndicator::Bars`, a thin line beside the
  code: the tag already says the whole file is new or gone, and a solid fill would drown the
  syntax colors. Modified and renamed files keep the fill and the `+`/`−` signs.
- `LoadedReview::files` is in patch order; the tree's `file_order` is the navigation order.
  `line_maps` and the parsed files are index-aligned; if the counts ever differ the review
  still loads but diff comments are disabled (`comments_aligned`).
- Hidden files: the patterns from settings (all repositories, then this repository) mark
  matching files untracked (`hidden_mask`): they are out of the progress, change and file
  navigation (`nav_order`), auto-mark and the "first unviewed" pick. They are left out of
  the tree unless "Show hidden files" is on, in which case they appear muted and italic and
  a folder holding only hidden files is muted too. The Files header's eye button opens the
  view options: tree or flat list (`TreeMode`; the list sorts by path and shows each file's
  folder beside it, and navigation follows it), compact single-child folders, show hidden
  files, and, while hidden files are not in the tree, a scrolling list of them with the
  part of each path its pattern matched highlighted; a click shows that file. "Edit hidden
  files…" there or in the row menu emits `ReviewEvent::EditHiddenFiles`, which opens
  Settings scrolled to the Hidden files group. The review's own `tree` is rebuilt from
  `LoadedReview::tree` (reused when the shape is the default and nothing is left out)
  whenever the patterns or the view options change; if the shown file becomes hidden the
  next unviewed tracked file opens, and when files reappear after everything was hidden
  one opens again. Tree selection survives a rebuild by node path. The row menu adds a
  pattern (this file or folder in this repository, the file's family such as `*_mock.go`
  here or everywhere, a folder name everywhere).
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
  for at least `AUTO_MARK_DWELL` (0.5 s); each decision is logged at debug level, so running through the tree with j/k marks nothing,
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
  splits its content width at the split ratio and wrapping pins that width to the viewport,
  so the two columns always share exactly the viewport. With Unified, long lines scroll
  horizontally, whole files too. Wrapping follows the setting rather than the mode shown, so
  a file looks the same as its neighbours under one setting.
- The split divider: the app draws the handle and passes `Diff::split_ratio`. The boundary
  is the original column's right border, at `viewport · ratio` from the body's left edge;
  the body sits inside the `Diff`'s 1 px border, so with `W` = area width − 2 the line covers
  `[W·ratio, W·ratio + 1)` in `diff-area` coordinates and the handle is centred there
  (`DIFF_BORDER`). `W` comes from a `canvas` in `diff-area` that stores its width
  (`split_width`) after layout, the same one-frame lag as the component's own viewport. The
  handle shows only when the file is in two columns (mode Split, text with changes) and `W`
  fits two `SPLIT_MIN_COLUMN`s (160 px). The ratio shown is `fit_split_ratio`: each column at
  least 160 px and within the component's 0.1–0.9, 50/50 when narrower; the saved
  preference is not rewritten by that fit. The ratio goes to the `Diff` only in Split mode:
  it is part of the component's layout key, and without soft wrap every change re-measures
  the whole file, which a width-dependent fit would trigger on every frame of a resize. A
  drag is relative to its start: mouse down stores the start x and the ratio shown
  (`split_drag`), each move sets `(W·start + dx) / W`, so the boundary never jumps under the
  cursor; mouse up anywhere clears it through `diff-area`, which outlives a handle that
  disappears mid-drag. The handle blocks
  mouse input to the code under it (not scrolling), shows a `drag_border` line on hover and
  while dragging, and a double-click resets to 0.5. `resize_split` saves the ratio 400 ms
  after the last move, detached, like the tree width.
- With "ignore whitespace", line maps come from a second patch without `-w`, matched by path,
  so comment positions follow GitLab's real diff rather than the whitespace-blind one.
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
- Added and deleted files show as one whole file, never as a split with an empty side, and
  as plain highlighted code: no change fill, a thin colored bar instead.
- No file header row above the code; the toolbar carries the file, as in IntelliJ's diff
  viewer.
- The split divider is movable; its ratio is one app-wide setting, not per file or
  repository.
- "Viewed" has one control, the toolbar button.
