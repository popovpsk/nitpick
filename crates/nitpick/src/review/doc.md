# review

One review: a branch comparison, a merge request (a GitLab merge request or a GitHub pull
request, through `forge`) or the uncommitted changes of the work tree, loaded into a file
tree, a diff of one file at a time, per-file "viewed" marks, an optional blame of the shown
file and, for a merge request, its discussions.
`ReviewView` is the entity; the window creates one per opened review.

Not here: choosing what to review (`workspace/start.rs`), the git operations
(`git_repo`), the tree and viewed models (`review_core`), the hosts' APIs (`forge`, which
wraps `gitlab` and `github`; nothing here names those crates).

## Files

- `mod.rs` — `ReviewView`: loading (`load_review`), installing results, viewed marks and their
  save, tree selection and collapse state, file navigation, pull, discussions and the comment
  composer; actions and key bindings.
- `model.rs` — `ReviewSource`, `LoadedReview`, `ReviewFile` (with both blob ids), `build` (the range's commits,
  optionally narrowed to one of them; changed files + full context patch, kept as text with
  each file's section (`review_core::file_spans`); line maps (plus the shown patch's map
  with "ignore whitespace"), fingerprints, tree), `LoadedReview::diff_file` (parses one
  file's section into a `DiffFile` with `review_core::language_override` applied),
  `MrContext` / `MrRefs`, `forge_remote` / `ForgeRemote`, `resolve_mr_refs`,
  `comparison_for_mr`; `interdiff` (one file's diff from the
  version viewed before to the current blob, `Interdiff`).
- `model_grammars.rs` — `register_highlight_queries`: highlight queries for grammars
  gpui-component 0.9.1 links without one (C#, Swift, CMake from their grammar crates; C++ as
  the C++ query plus C's; proto and GraphQL from `../../queries/*/highlights.scm`), and the
  `vue` language gpui-component lacks (`tree-sitter-vue-next` with our own
  `../../queries/vue/` queries: the crate's are nvim-treesitter's, with `inherits` and
  `lua-match` predicates the component's highlighter does not run). `<script>` and every
  template expression (`{{ }}`, directive values) are injected as TypeScript, which also
  parses plain JS, since a query cannot say "no `lang` attribute"; `<style>` as CSS.
  `needs_injections` / `injecting_highlighter`: the `Diff`'s own highlighter skips injected
  languages, so a Vue file is highlighted through `SyntaxHighlighter::styles` instead (a
  fresh highlighter per hunk: reusing one would parse against a tree from other text).
- `model_tests.rs` — the whole pipeline on a throwaway repository; every enabled language
  produces syntax colors.
- `viewed.rs` — `SharedViewed`: one `ViewedStore` entity per repository for the whole app,
  filled from `app_db` in the background; each mark is written to the database at once.
- `tree_panel.rs` — the file tree panel (`uniform_list` of precomputed `TreeRow`s) and its
  keyboard handling.
- `diff_panel_tests.rs` — where the tree header drops the count and shrinks the bar.
- `diff_panel.rs` — the review header (branch pickers with pull buttons, the MR, or the work
  tree; the commits button), the viewed progress widget the tree header shows
  (`render_viewed_progress`), the file toolbar (change and file navigation, the shown file's
  label as a cached `FileLabel`, open-file, viewed and layout controls) and the `Diff` element with its
  movable split divider; the sliding, resizable tree panel; the assistant panel's place in
  the body; `Render for ReviewView`.
- `agent/` — the review assistant (`AgentPanel`, an entity `ReviewView` holds: the panel, the
  conversation and the harness session of the review); see `agent/doc.md`. `ReviewView` feeds it
  through `sync_agent`, which hands over a `ReviewContext` after an install, a discussions load,
  a viewed-marks change and a hidden-files change. A single-commit view is skipped, so the
  assistant always describes the whole review.
- `branch_picker.rs` — `BranchPickers`: one fuzzy `Picker` per side of a branch review, the
  branch list loaded when one opens, and the pick → `ReviewEvent::Open`.
- `commit_picker.rs` — `CommitPicker`: the header's commits button state (the chosen
  commit's label and tooltip) and its fuzzy `Picker`, entries built when it opens, and the
  pick → a reload of the same review at that commit.
- `avatars.rs` — the `Avatars` global (`init`): an in-memory map from avatar URL to the
  decoded picture (loading, ready or missing), filled by `request` (disk cache through
  `avatar_cache`, else a download on `net_runtime` with the matching forge's token, a stale
  entry when that fails); `render` draws the picture or, until it is there, initials on the
  author's tint (blame's `tint`, seeded by the username). Finished loads refresh the windows
  once per 100 ms batch. A picture that failed for a temporary reason is tried again the
  next time discussions load; any other failure (401 from a sign-in-only GitLab, not a
  picture) stays initials until the next launch, logged once.
- `comments.rs` — discussion cards and the composer as `Diff` annotations, also used on the
  sheet. Remote markdown (notes, the description) goes through `remote_markdown`: a link
  opens only for http, https or mailto (`opens_in_browser`; a `file:` or custom-scheme link
  in someone's comment could launch a program), and images resolve as URIs only, so a
  `data:` image is never decoded (gpui-component would decode it inline with no size cap). A card's top-left chevron collapses it to one line (avatar, author, preview of the
  first note, reply count, resolved tag); resolved threads start collapsed, open ones
  expanded, and `thread_collapse` keeps the user's explicit choice per thread, so resolving
  or unresolving doesn't flip a card the user set. A card with its reply composer or a note
  editor open is never drawn collapsed, so the focused textarea stays on screen. Each note: avatar,
  name, meta (`@username · time`, the time converted from the host's RFC 3339 stamp to the
  system time zone by `note_time`), and for the user's own notes (`NoteView::own`, matched on the username from
  `current_user`) edit and delete buttons at the right end of the header, shown on hover of
  that note; editing swaps the body for a textarea with Cancel / Save (⌘/Ctrl-Enter). A long
  body (`NoteView::long`: over 10 lines counting a wrap every 90 chars) is cut to about ten
  lines with a fade and a Show more / Show less toggle (`expanded_notes`). Starting another
  edit replaces the open one; a reload that no longer has the edited note drops the editor.
- `comment_list.rs` — the comments sheet (titled Comments; its body is the `CommentsSheet`
  view, embedded `.cached()` and redrawn when the review notifies, since the sheet builder
  runs on every window frame; when focus is lost inside it (a posted reply's input removed,
  the sort menu closing without restoring focus) it refocuses the closest focusable ancestor
  still drawn, if that is inside the body, since the sheet's close button dispatches `Cancel`
  from the focused element and with nothing focused it does nothing; a dialog closing over
  the sheet is not inside the body, so its deliberate blur is left alone) with a Description tab first when
  the request has one (its markdown body, `ReviewView::description`) and two more: On code (every
  discussion with a file position, outdated ones included, as full cards: open threads first,
  then resolved, each section in the chosen order, the file named whenever it changes from
  the card before; above each card its
  `L<line>`, an outdated tag when its `head_sha` is not the review's head, and Go to code
  (Go to file when the thread is not drawn in the diff, since nothing to scroll to), which
  closes the sheet and `reveal_discussion` opens the file, at the line when the thread is
  drawn) and General (discussions without a file, with the new-comment
  composer). The order (`CommentSort` in the settings, a menu beside the tabs, hidden on
  Description): Newest first by each thread's last note (`DiscussionView::active`), so a
  new reply brings its thread up; Oldest first by its first note (`created`), as GitLab's
  default; By file, tree order then line, On code only (General has no files, so there it
  is Oldest first and the menu offers the two time orders; picking the order already shown
  there keeps a saved By file). `build_code_threads` and
  `general_threads` precompute the rows, again when the setting changes; it opens on General when there is no
  code thread, and on Description when there is no comment at all, until the user picks a tab
  (`comments_tab_chosen`); from then on it opens on that tab for this review.
- `review_tests.rs` — comment times in a fixed time zone; which file Viewed advances to.
- `comment_list_tests.rs` — previews and row order in each sort.
- `note_actions.rs` — `NoteEdit` and edit / delete (a danger confirm dialog) of the user's
  own notes, then a discussion reload; a resolve toggle is busy per thread (`resolving`) so a
  second click can't repeat it; `ApprovalView` (label, tooltip with who approved) and
  `toggle_approval` for the header's Approve button, reloaded with the discussions; the button
  stays busy until the reloaded approval arrives, so a second click can't repeat the call.
- `pending.rs` — the review in progress (`Drafts`): the host's pending review loaded with
  the discussions (`load_pending`), drafts indexed by file and by the thread they answer
  (`index_drafts`; a line draft whose head is not the loaded one is listed as outdated and
  not drawn), the draft card (`render_draft`, edit and delete), the review sheet (drafts as
  full cards grouped by file in tree order, then line, general ones first — `Drafts::sheet`,
  built with the index; each with its line, the thread it answers, go-to, edit and delete;
  the summary, the verdict tabs, Submit and Discard), and the
  composer's guards (`can_draft`, `submit_primary`, `has_unsent_comment`, `reveal_composer`,
  `request_cancel_composer`, `confirm_discard_comment`).
- `push_watch.rs` — `PushWatch`: whether the request got new commits since it was loaded,
  and the banner offering Refresh.
- `blame.rs` — `ToggleBlame`, `BlameState` (the cache, the shown marks, the fetch), the
  toolbar toggle (`render_blame_toggle`) and the chips drawn in the `Diff`'s marker lane
  (`BlameMarks::render_chip`).
- `file_search.rs` — `FileSearch`: the Files header's sliding filter field, its ranked rows
  and their rendering in place of the tree.
- `find_bar.rs` — `FindBar`: find in the shown file (the text of both sides read off the UI
  thread, line hits as `Diff` decorations, the overlay bar).
- `since_viewed.rs` — the "since last view" mode: which files were viewed at an earlier
  version (`seen_before`), loading and installing the `Interdiff`, the toolbar toggle and
  the tree badge.
- `file_actions.rs` — `CopyPath`, `OpenInEditor` (a target outside the checkout, through a
  symlink, is refused; with no editor command a `launchable` file — a script, binary, app
  bundle or shortcut — is revealed in the file manager, `reveal_invocation`, instead of being
  handed to the OS, which would run it; on Windows every file is, since file associations
  there run interpreters for many types, `reveal_instead`): the file items of the tree row menu and the
  toolbar path menu (copy path, open in editor, open in GitLab / on GitHub; the items carry
  their action only so the menu shows its key — gpui-component runs an item's own click
  handler instead of the action, so the row menu still acts on the row), the editor
  launch, the project's web address (`WebLink`), and their pure parts (`editor_invocation`,
  `substitute_shell`, `find_program`, `project_web_from_mr`, `project_web_from_remotes`,
  `blob_url`, `change_line`, `pick_line`).
- `file_actions_tests.rs` — the pure parts, the `PATHEXT` lookup on a `TempDir`, and real
  `/bin/sh` runs (`HOME` in the temp dir) that get an awkward file name through every
  quoting context unmangled.
- `image_preview.rs` — `ImagePreview`: the before/after panes shown in place of the `Diff`
  for a binary image, their off-thread loading and decoding, and their rendering. A GIF or
  WebP is shown as its first frame (`first_frame_png`): gpui keeps every frame as a
  full-size buffer, so a small file of many frames would otherwise exhaust memory.
  `read_regular_file` (also used by `find_bar.rs`) reads a work-tree file only when it
  resolves to a regular file, and at most the caller's limit plus one byte: a symlink to
  `/dev/zero` or a FIFO in the work tree would otherwise never finish or never return.
- `image_preview_tests.rs` — `load` on a throwaway repository with gpui's real (CPU)
  `SvgRenderer`: a plain SVG still renders, one naming a local file is refused, a work-tree
  folder is never read.

## Spec

- Diff content: `install_content` is the one place a file's diff goes into the `DiffState`:
  the review's own file (`diff_file`, parsed on the UI thread when it is shown), or the `Interdiff` when "since last view" applies to the
  file. While an interdiff loads the state is empty (`content_pending`), and change steps
  wait; a cross-file step remembers its direction (`pending_jump`) and lands once it is in.
- "Since last view" (`since_mode`, view state, off by default): a file whose current
  fingerprint is not marked viewed in the store but an older version of it is
  (`ViewedStore::viewed_versions_except`, newest first, skipping versions with the current
  new blob, which would have nothing to show) is "seen before": it gets a clock badge
  in the tree while unviewed, and the toolbar shows the toggle while it is shown (viewed or
  not). With the toggle on such a file shows `git diff <viewed new blob> <current new blob>`
  (full context, the review's whitespace setting, named after the file), or a "nothing
  changed" state when the blobs match; comments, line discussions and blame are off for it,
  and find and change navigation follow it. Not for the work-tree review (its blobs are
  hashed without being written) or binary files; a failed comparison (a blob gone after a
  force push and gc) notifies once and drops that version from the candidates.
- Files filter: the search button in the Files header (`FindFile`, ⌘/Ctrl-P) slides out a
  field under the title; with text it replaces the tree with files visible in the tree,
  ranked by `review_core::rank` over (name, path) and drawn with the matched characters
  highlighted in the name and the folder. Up/Down move and show the file, Enter shows it
  and focuses the diff, Escape closes the field; with an empty field Up/Down move in the
  tree. Rows are ranked when the text or the tree changes, never in render. The field sits
  outside the `ReviewTree` key context, so letters typed there never reach the tree's
  single-letter bindings; the tree's focus, context and mouse-down focus live on the list
  below it.
- Find in file (`FindInFile`, ⌘/Ctrl-F): an overlay bar at the top right of the diff.
  The text of both sides of the shown content (blobs, or the file on disk for the work
  tree's new side; up to 16 MB) is read off the UI thread when the bar opens and when the
  shown content changes; matching is case-insensitive and line-based over
  `LineMap::rows()` (a context line counts once), every hit line is tinted with a
  `DiffDecoration` (both sides of a context line, ranges merged) and the current one more
  strongly, since the `Diff` has no inline search highlight. Typing jumps to the first hit;
  Enter / Shift-Enter step with wrap-around; a jump scrolls to the hit with up to three
  shown rows above it (`LineMap::shown_lines`) so a folded line is never unfolded just for
  the lead. Escape or the close button closes it and focuses the diff. The bar stays open
  across file switches and recounts.
- Whole files (`show_whole_file`): the `DiffState` gets `context_lines(None)` instead of
  3, so nothing is folded; blame chips are recomputed for it. Opening a file then lands on
  its first change (the change cursor is on it, so Next goes to the second), as IntelliJ
  does, instead of the top of the file. Wrapping (`wrap_lines`): split
  view only (the default, as before), always, or never; with no wrapping in split view each
  column clips and both scroll together.
- Progress reset: the tree header's progress opens a menu with "Reset viewed progress…"; after a
  confirmation every viewed tracked file of the review (the ones the progress counts) is unmarked and forgotten as "kept unviewed",
  so auto-mark works on them again.
- What a branch review shows is `git_repo::Repo::compare`: the merge base of the source with
  the target and the target's upstream, so commits the target gained since never show up.
  A merge request shows the host's diff refs: GitLab reports the merge base (`base_sha`), so
  the review is `base_sha`..`head_sha`; GitHub reports only the target tip (`start_sha`) and
  `head_sha`, so `resolve_mr_refs` takes `git merge-base` of the two locally (`MrRefs`), which
  is the three-dot diff GitHub shows. When a needed commit (`forge::DiffRefs::needed_commits`:
  head and the merge base, or head and the target tip) is missing, `Request::refspecs` are
  fetched (GitLab: `refs/merge-requests/<n>/head` into `refs/nitpick/mr/<n>`; GitHub:
  `refs/pull/<n>/head` and the target branch into `refs/nitpick/pr/<n>/{head,base}`, hidden
  from branch lists) from the remote whose URL path is the project, else `upstream` on the
  same host as `origin`, else `origin`, else any
  remote on the host (`forge_remote` with just that host).
- Comments go through `forge::Client` with the request's own number and `MrRefs`. A card
  offers Reply only when `can_reply` (GitHub conversation comments take none) and
  Resolve/Unresolve only with a `resolve_id` (GitHub: a review thread the viewer may toggle,
  from GraphQL; none when GraphQL failed), so no button is shown that cannot work. GitHub
  accepts a line comment only inside its own diff hunks, so for it a line farther than
  `ForgeKind::comment_context` from a change (`LineMap::near_change`) opens no composer and
  says the line cannot be commented on. The UI names a request by `Request::reference` (`!5` / `#5`).
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
- Diff highlighting: `set_diff_files` installs `injecting_highlighter` on the `DiffState`
  while the shown file needs injections (Vue) and clears it otherwise, so every other
  language keeps the component's highlighter, whose colors resolve at paint. Its styles are
  fixed when computed, so a `Theme` change (theme, syntax palette, preview) installs it again
  with the new `highlight_theme`. The patch has full context, so a file side is one hunk and
  `<script>` reaches its tag; in the 3-line fallback for huge reviews a hunk inside
  `<script>` is parsed as template text.
- The file toolbar, left to right: tree toggle (at the right end instead when the tree is on
  the right, so it sits next to the panel it opens), previous/next change, previous/next file
  (`PreviousFile`/`NextFile`, disabled without a visible neighbour), `n / m` in tree order,
  then the file: status icon in the tree's colors, the path with the directory muted and the
  name in the foreground color, ellipsized from the start so the name stays, `+a`/`−d` when
  non-zero, and a "new file" / "deleted" / "renamed" tag (the renamed tag's tooltip names the
  old path), then Open file right after them, as part of what names the file (a menu: open in
  editor and copy path with their keys, open in GitLab / on GitHub); the cross-file hint
  follows. On the right: the Viewed split button (a
  checkbox-style toggle, green once viewed, with `ToggleViewed`'s key in its tooltip; its caret
  holds the two marking behaviours as checkable items: mark on leave (`auto_mark_viewed`) and
  go to the next file (`advance_on_viewed`), which live nowhere else), blame, and the
  split / unified layout, the only place to change it. Every button whose action has a key
  shows it in its tooltip (`tooltip_with_action`).
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
  `line_maps` and `file_spans` are index-aligned; if the counts ever differ the review
  still loads but diff comments are disabled (`comments_aligned`). Only the text and per-file
  metadata are kept, since a parsed `DiffFile` is many times larger than its patch text: the
  review parses every section once at load for its counts and paths, drops it, and parses the
  shown file again on demand, keeping that one parse (`parsed`) so reinstalling the same file
  costs nothing. That parse runs on the UI thread, a pass the `DiffState` makes over the file
  there anyway: off-thread it would leave the state empty and every line jump pending, as an
  interdiff does. A section that does not parse to exactly one file fails the load, so
  `diff_file` failing later is only logged. With "ignore whitespace" the exact patch's maps
  are matched by path in order, since a type change (file ↔ symlink) gives two sections with
  the same path.
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
  Settings on its Hidden files page. A hidden file shown in the tree shows its own viewed
  state; from a hidden file navigation continues with the nearest tracked file in the
  order on screen (the flat list's path order in List mode). The review's own `tree` is rebuilt from
  `LoadedReview::tree` (reused when the shape is the default and nothing is left out)
  whenever the patterns or the view options change; if the shown file becomes hidden the
  next unviewed tracked file opens, and when files reappear after everything was hidden
  one opens again. Tree selection survives a rebuild by node path. The row menu adds a
  pattern (this file or folder in this repository, the file's family such as `*_mock.go`
  here or everywhere, a folder name everywhere).
- Viewed marks key on `path` + `old_blob..new_blob`. Every review of a repository, in any
  window, shares one store entity and observes it, so marks made in one window show in the
  others and no window overwrites another's marks with a stale copy. The entity starts empty
  and fills when the database answers (the review's observer re-reads it); marks made before
  that are written at once and replayed onto the loaded map, so none is lost. Each change is
  one queued database write; quitting awaits `db::barrier`. Marking the current file viewed jumps to the next unviewed
  file in tree order when `advance_on_viewed` is set, or, with none below, to the nearest
  unviewed one above (`next_unviewed_after`), never back to the top: a reader going upward
  stays where they are; a folder toggles all files under it.
  With `collapse_viewed_folders`, a mark that completes a folder (hidden files not counted)
  collapses it and every completed folder above it; the selection moves to the row that now
  shows the selected file. Unmarking never expands anything.
- A discussion is placed on a line only when its position's `head_sha` is the review's head;
  others are listed as outdated on the sheet's On code tab. A new line
  comment sends `old_line`/`new_line` from `LineMap::locate`: both for a context line, one
  for an added or removed line.
- Tree rows are rebuilt when data changes (load, viewed, collapse, discussions), never in
  `render`; the list closure only reads them.
- Default keys live in `keymap.rs` and can be changed in Settings; the contexts below are
  what the registry binds. Key bindings in the tree (context `ReviewTree`) are single letters (`j/k/h/l`, `space`/`v`,
  `n/p` for the next/previous file); anywhere in the review (context `Review`) they are
  `space` (viewed) and modifier or function keys (`alt-up/down` and `f7`/`shift-f7` for the
  next/previous change, `alt-left/right` for the previous/next file, `cmd-b`, `cmd-r`,
  `cmd-1/2`, `cmd-f` find in file, `cmd-p` filter files, `cmd-shift-c` copy path, `f4` open in editor). `space` and the `alt-` arrows pass through (`cx.propagate`) while an input has
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
  gpui-component's `DiffState`, hence the cursor and the scroll detection.
- Auto-mark (`auto_mark_viewed`): leaving a file for another marks it viewed if it was shown
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
  of that same composer closes it. Cancel asks first when it holds text, and so do the
  workspace's review switch, window close and Quit — those also count an edited draft and a
  typed review summary (`has_unsent_comment`). While the composer holds text another line
  or Reply does not open a second one: `reveal_composer` scrolls to it (a reply to a thread
  not drawn in the diff opens the comments sheet on its tab), focuses it and says why.
- A merged or closed request is read-only (`request_open`): the header tags its state, and
  no composer, reply, resolve, own-note edit, draft or new-push check is offered; its
  discussions still show.
- Review drafts (`pending.rs`): where the host takes a draft for the target (`can_draft`:
  any line; a reply to a thread that takes replies; a general comment only on GitLab), the
  composer's primary button and ⌘/Ctrl-Enter add it to the review ("Start a review" before
  the first, "Add to review" after) and "Comment now" posts at once; elsewhere the primary
  button posts. While a GitHub review is pending, GitHub takes no single line comment, reply
  or approval outside it, so "Comment now" is hidden there (`direct_blocked`) and Approve
  opens the review sheet with Approve chosen. The resolve checkbox (GitLab, an open
  resolvable thread) resolves the thread on submit for a draft, or right after the reply
  for "Comment now". Drafts show as cards in the diff (line drafts) or under the
  thread they answer, tagged Draft; the header's "Review · N" button (shown while a review
  is pending) opens the sheet. The verdicts offered are the host's, Approve and Request
  changes only when the user may approve; Submit needs a draft, a summary or a verdict other
  than Comment (on GitHub, Request changes needs a summary). A submit, success or not,
  reloads the discussions (a GitLab submit is several calls and may stop halfway). Drafts load
  with every discussion reload; a failed first load (an old GitLab without the draft notes
  API) leaves `pending` empty and the composer posts directly, and a failed later load keeps
  the drafts already shown. The verdicts are fixed when the sheet opens (`ReviewForm`), and
  the header label and the composer's key hints are built when their data changes, not per
  frame. The composer's footer wraps: in a narrow card (the sheet) the key hint keeps one line
  and the buttons move below it instead of squeezing it to a letter-wide column.
- New pushes (`push_watch.rs`): while a merge request's window is active, at most every
  5 minutes (a 30 s tick checks the time, and window activation checks at once) one request
  fetch compares the head with the loaded one; nothing is fetched in an inactive window. A
  new head shows a banner under the header until a reload installs it; the diff is never
  replaced on its own, so the text does not move under the reader. Posting a comment checks
  at once, whatever the time since the last check: GitLab moves a line comment made against
  an older version onto the request's latest diff, so its head no longer matches the loaded
  one and it is listed (as outdated) but not drawn in the code until a reload; the banner
  offers that reload. A submitted review checks at once too, for its line drafts.
- Starting a line comment (`can_comment_lines`): no `on_add_annotation` button (it trails
  the pointer over the code and is lost when a press clears the selection it hangs on).
  Instead the hovered line gets one gutter marker (`comment_marker`, set by
  `set_hovered_line` when the hovered line changes, which then redraws): a faint comment icon
  in the marker lane, right of the line number (the lane is the only slot the component
  offers), no fill; a click opens the composer. The icon shows only while the pointer is on
  the row's left padding, line numbers or lane, not over the code: the marker spans that
  strip with a transparent group element (`comment_hot_zone` repeats the component's gutter
  geometry, sized for at least `GUTTER_DIGITS` digits since the file's widest number is not exposed; it takes no input, so line-number
  presses still reach the gutter) and the icon is drawn on that group's hover. The marker
  itself stays on the hovered line, so the lane is never empty when the icon is wanted. Not a marker per line: the component rebuilds
  its marker map from the list on every frame. A `COMMENT_LANE` marker at line 0 (no such
  row) keeps the lane while nothing is hovered, so the code does not shift under the
  pointer. Blame markers go first, so a line with a blame chip shows the chip. Lines GitHub
  would reject (`line_commentable`) get no icon, and a click on their number explains the
  3-line rule in an info notice.
  A line-number gesture opens the composer when the mouse is released: `SelectionStarted`
  sets `gutter_gesture` and the next `SelectionEnded` takes it, so a Shift-arrow selection,
  which ends without starting, opens nothing. `AddComment` (C) comments the selection, else
  the line under the pointer (`hovered_line`, from `on_line_hover`). Leaving one line reports
  after entering the next (handlers run in reverse paint order), so a line's `None` within
  `HOVER_SWAP_WINDOW` of an enter is ignored; a later one (the pointer moved onto a card, a
  hunk header or below the code) clears the line and its icon, as does leaving the diff area
  or installing a review). Opening the composer clears the selection. The On code tab's
  empty state names both ways.
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
- Blame (`settings.show_blame`, the toolbar toggle and `ToggleBlame`, no default key; the
  setting is global, so every window follows it): the shown file is blamed off the UI thread
  on both sides in one task — the new side at the review's head commit (the work tree for
  the uncommitted-changes review), the old side at `base_sha` under the old path — skipping
  the side an added or deleted file lacks. One side failing still shows the other; only both
  failing counts as a failed blame. A fetch starts `BLAME_DELAY` (120 ms) after the
  file is shown, so running through files with j/k starts no `git blame` (a started one runs
  to its end: the blocking child is not killed when its task is dropped). Results are cached
  by both sides' (rev, path) for the installed review; a reload that installs a new review drops the
  cache and refetches, and an unchanged reload keeps it like everything else but retries
  failed blames. Chips are shown only for the file and content they were computed for: the
  same key with the same fingerprint keeps its chips until the refetch lands (a work-tree
  reload does not blink); anything else shows an empty lane until its result arrives, and a
  result that lands after the file changed is cached but not shown. A failed blame leaves
  the lane empty and shows one notification until the next reload or toggle.
- Blame chips use the `Diff`'s marker lane (between the line numbers and the code, one row
  high and one row wide, in each column in split mode, once in unified): no extra row. In
  split mode each column gets its own side's chips, so the lane the component opens on the
  left is never empty; unified mode shows the new side only (the old side for a deleted
  file), since a context row would otherwise carry two chips (`BlameMarks::markers`). A
  chip sits on the first line of each run from one commit and on the first line of each
  block the default folding shows (`review_core::LineMap::shown_block_starts`). It is the author's initials on a tint picked from the commit id
  (`review_core::blame::tint_index`) over six theme hues at two strengths with the theme's
  foreground, so it reads in light and dark themes; uncommitted lines get a muted "you". The
  tooltip shows author, relative date (computed when it opens), short id and summary. While
  blame is on, the lane stays even before results arrive or when there are none (a
  placeholder marker on no line), so the code does not shift sideways on every file switch.
  Markers and chip data are built when a result lands; render only clones the marker list
  the component's builder takes and looks a chip up by binary search.
- With "ignore whitespace", line maps come from a second patch without `-w`, matched by path,
  so comment positions follow the host's real diff rather than the whitespace-blind one;
  `ReviewFile::display_map` keeps the shown patch's map (`shown_map`), from which `changes`
  and blame's block starts are taken.
- A discussion whose position has no line (file or image comments) is listed on the On code
  tab without a line; one with no file at all on General. Tree rows show comment counts (a
  collapsed folder: the sum of the files under it) unless `tree_comments` is off, a switch in
  the tree's view options (the eye). A viewed file is tinted green with a check; striking its
  name through is another switch there (`strike_viewed`, off by default).
- Header buttons on the right, in order: refresh, comments (merge requests only), assistant
  (`Icons::Sparkles`, every review kind, while `agent.enabled`; `ToggleAgent`, ⌘/Ctrl-Shift-A).
  The assistant button spins while a turn runs and is highlighted while a finished overview
  waits unread behind the closed panel. The panel sits at the right edge of the body, after the
  diff and before a right-hand tree, and never covers the diff.
- Approve button (left of the refresh button): "Approve" (with the count when others
  approved), "Approved" in the success style once the user approved (a click withdraws on
  GitLab), or just the count when the user can't approve (their own request); hidden when
  none applies.
- Image preview: a binary file (`ReviewFile::binary`) whose existing sides all have an image
  extension (png, jpg/jpeg, gif, webp, bmp, svg, ico, tif/tiff: what gpui's `ImageFormat`
  decodes) shows "Before" and "After" panes instead of the `Diff`; an added file only After,
  a deleted one only Before. Each side's format comes from its own path, so a rename that
  changes the extension decodes both. Other binaries keep the `Diff`. An SVG git diffs as
  text stays a text diff. The panes sit side by side and stack when the diff area is
  narrower than `STACK_BELOW`; each image is scaled down to fit its pane, never up
  (`ObjectFit::ScaleDown`, one image pixel per logical pixel), with its pixel size and byte
  size under it. A side is read with `Repo::blob` (the work-tree review reads its new side
  from disk, since `hash-object` there writes no object); over `MAX_PREVIEW_BYTES` (20 MB,
  checked with `blob_size` or the file's metadata before reading) it says too large, and a
  read or decode failure says so in the pane and logs a warning. An SVG goes through
  `review_core::check_svg` before gpui renders it: gpui's renderer uses usvg's default
  options, which read any file path an `<image>` or `<feImage>` names (`/dev/zero`, a UNC
  share on Windows) and unpack svgz without a limit, so an SVG in someone's merge request
  could hang the app or leak the user's Windows credentials. The same check caps pictures
  embedded in the SVG at `MAX_PREVIEW_PIXELS`, as for a raster file; refused, it shows the
  failure text. Reading and decoding run
  on the background executor, one task per side; the state is built when `show_file` puts a
  new file in and when a side lands, keyed by path + fingerprint (an unchanged reload keeps
  it) and a generation, so a result for a file the user has left is dropped. Only the shown
  file's images are kept; replaced ones are removed from the sprite atlas (`drop_image`,
  deferred so every window is free), as are the last ones when the review is released.
- Header branch chips: the name opens a fuzzy branch picker for that side only (the branch
  list is read with `git for-each-ref` each time a picker opens, off the UI thread);
  picking emits `ReviewEvent::Open` with that side replaced and the other kept, and the
  window opens it as a new review exactly like the start screen does, recording it as the
  repository's last review. Picking the other side's branch, or the current one, does
  nothing.
- One commit: branch and merge request reviews have a ghost commits button in the header
  (after the branch chips, or after the MR's "source → target · author"; none for the work
  tree or an empty range). It reads "N commits" (tooltip: the merge base and the count for
  a branch review), or the chosen commit's short id and subject cut to 48 characters
  (tooltip: subject and author). Its picker lists "All changes", then the range's commits
  newest first (subject; short id · author · relative time, searchable), the current one
  checked; entries are built when it opens, so times and language are current. A choice is
  `ReviewView::commit`, view state only: not saved, gone with the view. `build` then diffs
  the commit's first parent (the empty tree for a root commit,
  `Repo::first_parent_or_empty_tree`) against the commit, so `base_sha`/`head_sha` are that
  pair and blame runs at the commit; `commits` stays the whole range and `merge_base_sha`
  the range's base. A load whose range no longer holds the commit falls back to all changes,
  and `commit` follows what was installed (a failed load restores the installed one).
  `install` treats a different commit as a new review: no "unchanged" fast path, the shown
  file is not kept and the first unviewed file opens. In commit mode no discussion is placed
  on a line (all go to the sheet), the diff offers no line comment and a pending line draft
  is neither drawn nor sent, since comment positions refer to the request's diff. Viewed marks key
  on the commit's blob pairs; the full diff of a file counts as viewed once viewed pairs chain
  from its old blob to its new one (`ViewedStore::is_viewed`), e.g. every commit that changed
  it, a rename included, and unmarking it there removes those pairs (`covering_marks`). A pick during a pull or a post loads without taking `busy`, so it
  never clears theirs. The unchanged-reload check also compares the commit list and the
  merge base, since in commit mode a grown range keeps the same pair and files.
- Viewed progress leads the tree header (`viewed/total` and a thin bar, the count in the
  tooltip), read from `progress()`: it belongs with the files it counts, and the review
  header is left to the request (approvals, comments). The header has no "Files" title —
  the list says what it is. Its menu opens from its left edge. With the tree hidden it is
  not shown. As the tree narrows (`ProgressFit::for_width`, in rems so it follows the
  interface size, the count's width from its digits) the count goes first (the tooltip
  still has it), then the bar shrinks to a small floor; the progress box clips rather than
  overlap the header buttons. The header's button row is `TOOLBAR_HEIGHT_REMS` tall with its
  border, like the file toolbar beside it, so the two bottom borders line up at every
  interface size (in rems, as the buttons in them are); the filter
  field slides out below that row.
  The tree's view options popover opens toward the diff (from the button's left edge when
  the tree is on the left), so the files stay visible while the options change.
- File actions: the tree row menu starts with Copy path, Open in editor, Open in GitLab (or
  on GitHub), then a separator and the hide items; a folder gets only Copy path. Right-click
  on the toolbar's path opens the same items for the shown file, with their keys. Copy path
  writes the repository-relative path (the new one; the old one of a deleted file) and says
  nothing. Open in editor is not offered for a deleted file; the file is
  `root/new_path` in the work tree, checked on the background executor, and a missing one
  is an error notification, not a launch. Its line: for the shown file the change cursor's
  change, or the end of the diff's selection on the new side when the user scrolled or
  clicked since the jump (or there is no cursor), then the first change, then 1; any other
  file its first change. A change's line is its first added line, or for a pure removal the
  new line in its place (`anchor.new` plus the context lines between them), never the
  anchor itself, which sits up to two lines above the change.
- Editor launch (`settings.editor_command`): empty runs the OS opener (`open`, `xdg-open`,
  `explorer.exe`, whose exit code is ignored since it is 1 on success), which cannot jump to
  a line. Otherwise on macOS/Linux the template runs as `$SHELL -lc` (a login shell, so the
  PATH from the profile files, not the interactive rc files, reaches a GUI app; `/bin/sh`
  when `$SHELL` is not an sh-family shell or fish). `{root}` (the worktree that has the
  review's source branch checked out, found by `git_repo::worktree_for` when the editor
  opens, else the repository's work tree; the file is taken from the same checkout, so its
  lines may differ from the reviewed head),
  `{file}` and `{line}` become references to `NITPICK_ROOT`/`NITPICK_FILE`/`NITPICK_LINE`,
  passed in the environment, so the path never goes
  through quoting; the reference depends on the quoting around the placeholder
  (`substitute_shell`): `"$VAR"` outside quotes, `$VAR` inside `"…"`, `'"$VAR"'` inside
  `'…'`, so `{file}`, `"{file}:{line}"` and `'{file}'` all pass the path as one word. A
  template without `{file}` gets the file appended. On Windows the template is split on
  whitespace (double quotes group a word, so a quoted `Program Files` path works) and the
  values substituted per word; nothing goes through `cmd`, which would re-parse `&` or `|`
  in a file name: a bare program name is looked up on `PATH` with `PATHEXT`, so a
  `code.cmd` shim is spawned directly and Rust escapes its arguments. Spawned with null
  stdio on the background executor; the app never waits for the editor: it polls the child
  for `FAILURE_WINDOW` (5 s), reports a spawn error or a non-zero exit in that time as an
  error notification naming the command, then only reaps it. stderr is not captured, since
  a pipe dropped while the editor lives on would break its writes.
- Editor presets (`editor_presets`, the settings field's Presets menu fills the template):
  VS Code / Cursor `code {root} -g {file}:{line}`, Zed `zed {root} {file}:{line}`, JetBrains
  IDEs `<script> {root} --line {line} {file}`, on macOS `open -na "<App>.app" --args …`
  instead, which needs no command-line launcher. Each passes the repository first, so the
  editor opens (or reuses) it as a project and the file inside it, with its language
  servers, rather than as a lone file (JetBrains' LightEdit).
- Open in browser: the file at the review's head commit, `{project}/-/blob/{sha}/{path}#L{line}`
  (GitLab, and any host other than github.com) or `{project}/blob/{sha}/{path}#L{line}`
  (github.com, or a pull request's host), the path percent-encoded,
  the line as for the editor; a deleted file links its old path at `base_sha`, without a
  line. The project address is `WebLink`, set in `install` (never in render): an MR's
  `web_url` without `/-/merge_requests/<iid>` (GitLab) or `/pull/<n>` (GitHub); for a branch review the `origin` remote, else
  the first one, through `git_repo::parse_remote_url` (`https_base/path`, read off the UI
  thread). None for the uncommitted-changes review or when nothing parses, and then the item
  is not offered. The MR header's own open-in-browser button stays.
- Highlight queries are registered once, by the first `parse`, through gpui-component's
  `LanguageRegistry::register` (same grammar, new query); every highlighter is built after
  that, since files reach the `DiffState` only from a parsed review. The proto and GraphQL
  queries are ours, written against those grammars' node types; capture names must be
  gpui-component highlight names (a dotted name falls back to its prefix).

## Decisions

- Batched review as GitLab does it, drafts kept on the host, on both GitLab and GitHub; no
  multi-line comments for now.
- New pushes are polled, at most every 5 minutes and only in an active window; a banner,
  not an automatic reload.
- Comments sheet order is the user's choice (newest first, oldest first, by file),
  remembered app-wide; newest first is the default, since the new-comment button sits on
  top.
- One file at a time rather than all files in one scrolling list, so only the shown file is
  held parsed.
- Viewed progress is local; neither GitLab nor GitHub has an API for its "viewed" checkboxes.
- The uncommitted-changes review compares with `HEAD`, not with a target branch.
- Pull is fast-forward only and only from the branch chips of a branch review.
- Added and deleted files show as one whole file, never as a split with an empty side, and
  as plain highlighted code: no change fill, a thin colored bar instead.
- No file header row above the code; the toolbar carries the file, as in IntelliJ's diff
  viewer.
- The split divider is movable; its ratio is one app-wide setting, not per file or
  repository.
- "Viewed" has one control, the toolbar button.
- One commit is a view of the same review (the header's commits button), not a separate
  review kind; it is never persisted and disables line comments.
- Find in file tints whole lines, not the matched text, since the `Diff` exposes line
  decorations only; nothing in gpui-component is patched.
- "Since last view" is a toggle the reviewer turns on, not the default view of a changed
  file, and it never marks anything.
- Blame is compact: one initials chip per run of a commit in the marker lane, details in the
  tooltip; not Zed's per-line author and date column.
- Copy path, open in editor and open in browser live in context menus and key bindings, not
  in toolbar buttons.
- The assistant is fed by push (`sync_agent`) rather than by reading `ReviewView` from the
  panel: the panel is updated from inside `ReviewView`'s own updates, and reading the parent
  back there would panic.
- The assistant panel is non-modal like the tree panel, not a `Sheet` like the comments: the
  diff stays clickable beside it. Whether it is open is per-window state, closed at start and
  never saved, like the tree's visibility; only its width is a setting.
- Binary images show as before/after panes (GitLab's "2-up"), not swipe or onion skin.
