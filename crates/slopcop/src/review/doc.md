# review

One review: a branch comparison or a GitLab merge request, loaded into a file tree, a diff of
one file at a time, per-file "viewed" marks and, for a merge request, its discussions.
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
- `diff_panel.rs` — the review header (branches with pull buttons, or the MR), the file
  toolbar and the `Diff` element; `Render for ReviewView`.
- `comments.rs` — discussion cards and the composer as `Diff` annotations, and the sheet with
  general and outdated discussions.

## Spec

- What a branch review shows is `git_repo::Repo::compare`: the merge base of the source with
  the target and the target's upstream, so commits the target gained since never show up.
  A merge request shows GitLab's own `diff_refs` (`base_sha`..`head_sha`); when either commit
  is missing locally, `refs/merge-requests/<iid>/head` is fetched into `refs/slopcop/mr/<iid>`
  (hidden from branch lists) from the remote whose URL path is the MR's project, else
  `origin`, else any remote on the GitLab host.
- The patch is requested with full-file context so the diff can expand folded lines; over
  48 MB it is recomputed with 3 lines and the header says so.
- Exactly one file is in the `DiffState` at a time; selecting a file replaces it. This bounds
  highlighting work on huge reviews and keeps the file header a stable scope.
- `LoadedReview::files` is in patch order; the tree's `file_order` is the navigation order.
  `line_maps` and the parsed files are index-aligned; if the counts ever differ the review
  still loads but diff comments are disabled (`comments_aligned`).
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
- Key bindings in the tree (context `ReviewTree`) are single letters (`j/k/h/l`, `space`/`v`,
  `n/p`); anywhere in the review (context `Review`) they use modifiers (`alt-up/down`,
  `cmd-b`, `cmd-r`, `cmd-1/2`). `alt-up/down` pass through (`cx.propagate`) while an input
  has focus, so typing a comment never switches files. No `alt-<letter>` binding: on macOS
  bindings match before text input, so it would eat the character the layout types.
- The comment composer survives file switches and reloads; only Cancel or a successful post
  of that same composer closes it.
- Notifications go to the review's own window (`window_handle`) through `cx.defer`, never to
  whichever window happens to be active, and never inside that window's own update.
- The review observes the settings: a new diff layout applies at once, a change of "ignore
  whitespace" reloads.
- `render_diff` decides the `DiffState` mode for the file it shows: an added or deleted file
  is always unified (one column, whatever the layout setting), anything else follows the
  setting. It is a per-file decision because exactly one file is in the state; the toolbar
  keeps showing the saved layout. `apply_settings` / `set_layout` set the saved mode, and
  `render_diff` corrects it before the `Diff` draws, so no frame shows the wrong one.
- With the Split setting the `Diff` soft-wraps, whole-file views included: the component
  splits its content width in half and wrapping pins that width to the viewport, so both
  sides are always exactly half. With Unified, long lines scroll horizontally, whole files
  too. Wrapping follows the setting rather than the mode shown, so a file looks the same as
  its neighbours under one setting, and the header's right-hand label stays in view. The file header's "Added file" / "Deleted
  file" label is gpui-component's own.
- With "ignore whitespace", line maps come from a second patch without `-w`, matched by path,
  so comment positions follow GitLab's real diff rather than the whitespace-blind one.
- A discussion whose position has no line (file or image comments) goes to the sheet.
- Highlight queries are registered once, by the first `parse`, through gpui-component's
  `LanguageRegistry::register` (same grammar, new query); every highlighter is built after
  that, since files reach the `DiffState` only from a parsed review. The proto and GraphQL
  queries are ours, written against those grammars' node types; capture names must be
  gpui-component highlight names (a dotted name falls back to its prefix).

## Decisions

- One file at a time rather than all files in one scrolling list.
- Viewed progress is local; GitLab has no API for its "viewed" checkboxes.
- Pull is fast-forward only and only from the branch chips of a branch review.
- Added and deleted files show as one whole file, never as a split with an empty side.
- The split divider is fixed in the middle: gpui-component's `Diff` has no API for a column
  ratio, and a draggable divider would mean forking it.
- "Viewed" has one control, the toolbar button; the file header has none.
