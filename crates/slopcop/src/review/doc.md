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
  context patch → parsed `DiffFile`s, line maps, fingerprints, tree), `gitlab_remote`,
  `mr_refspecs`, `comparison_for_mr`.
- `model_tests.rs` — the whole pipeline on a throwaway repository.
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

## Spec

- What a branch review shows is `git_repo::Repo::compare`: the merge base of the source with
  the target and the target's upstream, so commits the target gained since never show up.
  A merge request shows GitLab's own `diff_refs` (`base_sha`..`head_sha`); when either commit
  is missing locally, `refs/merge-requests/<iid>/head` is fetched into `refs/slopcop/mr/<iid>`
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
  `n/p`); anywhere in the review (context `Review`) they are `space` (viewed) and modifier
  keys (`alt-up/down`, `cmd-b`, `cmd-r`, `cmd-1/2`). `space` and `alt-up/down` pass through
  (`cx.propagate`) while an input has focus, so typing a comment never marks or switches
  files. No `alt-<letter>` binding: on macOS bindings match before text input, so it would
  eat the character the layout types. Toolbar tooltips name their key through
  `tooltip_with_action`, so they follow the bindings.
- Auto-mark (`auto_mark_viewed`): leaving a file for another marks it viewed, except a file
  the user unmarked by hand in this review (`kept_unviewed`, cleared on every load since file
  indices change). Reloads and the first file of a load never mark anything.
- The tree panel is drawn here rather than with `h_resizable`, so it can slide: toggling
  animates its width over `TREE_SLIDE` (ease-out, as in pawse) while the content keeps its
  full width inside an `overflow_hidden` box, and a closing panel stays rendered until the
  slide ends. The first render of a review does not animate. The drag handle on its inner
  edge resizes it within `TREE_WIDTH_RANGE`; the width is saved to settings 400 ms after the
  last move.
- The comment composer survives file switches and reloads; only Cancel or a successful post
  of that same composer closes it.
- Notifications go to the review's own window (`window_handle`) through `cx.defer`, never to
  whichever window happens to be active, and never inside that window's own update.
- The review observes the settings: a new diff layout applies at once, a change of "ignore
  whitespace" reloads.
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

## Decisions

- One file at a time rather than all files in one scrolling list.
- Viewed progress is local; GitLab has no API for its "viewed" checkboxes.
- The uncommitted-changes review compares with `HEAD`, not with a target branch.
- Pull is fast-forward only and only from the branch chips of a branch review.
