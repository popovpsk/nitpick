# review_core

The review model the UI draws: the changed-file tree with folder compression and viewed
roll-up, the per-repository store of "viewed" marks, the map from a diff line to the
old/new line pair GitLab positions need, where blame chips go, the `diff:` links the review
assistant writes and the app follows, the fuzzy matcher every picker filters with, the syntax
language of paths the diff component does not recognise, and the check an SVG passes before
the image preview renders it. GPUI-free and git-free; only the store does file I/O.

Not here: producing the file list or the patch (`git_repo`), talking to GitLab or GitHub (`forge`),
drawing the tree or deciding when to save (`nitpick/src/review/doc.md`), the picker UI
(`nitpick/src/picker.rs`).

## Files

- `tree.rs` — `FileTree::build` from `TreeFile`s (`build_with` leaves out files a filter
  rejects and keeps the others' indices; `build_shaped` with a `TreeShape` builds a flat
  list sorted by path or a tree without folder compression; `progress_tracked` counts only
  the files a filter tracks), `rows` (the visible rows for a set of
  collapsed folder paths), `progress` (viewed/total per node), `file_order`, `ancestors`,
  `completed_folders` (the ancestor folders of some files whose progress is complete).
- `viewed.rs` — `ViewedStore`, the in-memory viewed map (`from_marks`, `is_viewed`,
  `set_viewed`, `covering_marks`: the marks that make a file count as viewed,
  `viewed_versions_except`: the other versions of a path marked viewed, newest
  first; persistence is `app_db`'s), `fingerprint`, `fingerprint_blobs`, `write_atomic` (owner-only on Unix from the first byte; the
  temp-file-and-rename write the JSON settings use).
- `line_map.rs` — `file_spans(patch)`: the byte range of each `diff --git` file section, in
  patch order; `line_maps(patch)`: one `LineMap` per `diff --git` file in patch order;
  `LineMap::locate(side, line)` → `LineKind` (context with both numbers, added, removed);
  `new_line_for_old(old)` → the new number of an unchanged old line (none for a removed one);
  `LineMap::changes()` → each run of changed lines (`Change`: its first line, and an anchor
  up to two context lines above it within the hunk, else the first line);
  `LineMap::rows()` → the patch's rows in display order (removed lines before the added
  lines of a change); `LineMap::shown_lines(side, context, min_folded)` → which lines of a
  side the diff shows with unchanged lines folded, and `shown_block_starts` → the first
  line of each such block; `near_change(line, context)` → whether a line would be inside a
  hunk of a diff with that many context lines (changed lines always are).
- `blame.rs` — `marker_lines` (the lines a blame chip goes on), `initials` of an author
  name, `tint_index` (a stable palette slot for a commit id).
- `blame_tests.rs` — marker placement, initials, tint stability.
- `diff_link.rs` — `DiffLink { path, range: Option<(Side, first, last)> }` and the `diff:`
  link format the review assistant writes: `parse`, `format`, `extract` (every `diff:`
  link target of a markdown text, in order, repeats kept), `DiffLink::locate(&LineMap)`
  (the first line of the range the patch shows).
- `diff_link_tests.rs` — every form, percent-coding and round trips, refusals, extraction
  from markdown, line checks.
- `fuzzy.rs` — `fuzzy_match(query, text)` → score and matched byte offsets; `rank` over
  `(label, optional detail)` items; `highlight_ranges` (offsets → byte ranges of whole chars).
- `language.rs` — `language_override(path)`: the highlighter language for a path by file
  name, extension, or a `Makefile.*` / `GNUmakefile.*` prefix (unless the extension names
  another language, as in cargo-make's `Makefile.toml`).
- `tests.rs` — tree shape and order, collapse, roll-up, store round trip and expiry,
  line-map cases including a removed line that looks like a `---` header, language cases.
- `fuzzy_tests.rs` — match positions, ranking order, highlight ranges.
- `hidden.rs` — `HideRules`: gitignore-style patterns (the `ignore` crate's matcher, rooted
  at the repository, global then repository lines so a repository `!pattern` re-shows a
  file); `matching_rule` names the pattern that hid a path; the pattern suggestions the tree
  menu offers (`file_pattern`, `dir_pattern`, `dir_name_pattern`, `family_pattern`), which
  `escape` glob characters and a leading `!`/`#` so `[slug]/` means that folder only;
  `merge_patterns` (an edit applied as a diff against the list it started from);
  `matched_ranges` (the byte ranges of a path that a pattern's literal characters matched,
  for highlighting: wildcards and classes are not highlighted; `{a,b}` alternatives are
  expanded, and failed sub-matches are memoized so `*` runs never backtrack exponentially); a rooted
  path (a drive-like first component on Windows) never matches instead of panicking.
- `hidden_tests.rs` — gitignore cases, re-include, bad patterns, suggestions.
- `svg.rs` — `check_svg(data, max_len, max_pixels)`: the SVG text the image preview may hand
  to gpui's renderer (svgz unpacked), or the `SvgError` that refuses it.
- `svg_tests.rs` — accepted and refused references, entity tricks, svgz limits, embedded
  picture sizes.
- `timestamp.rs` — `parse_timestamp`: an RFC 3339 / ISO 8601 date-time (`Z` or a `±HH:MM`
  offset, fractional seconds ignored) to Unix seconds, for "updated 3 h ago".
- `timestamp_tests.rs` — the formats GitLab and GitHub send, offsets, leap day, refusals.

## Spec

- Tree: folders before files, each level sorted case-insensitively; a folder whose only
  child is one folder is merged into it (`a/b/c`), as GitLab and Zed show them. Node indices
  are pre-order, so a child's index is always greater than its parent's — `progress` relies
  on it to fold in one reverse pass. Collapse state is a set of folder paths kept by the
  caller, so it survives a rebuild of the tree for a refreshed diff.
- `file_order` is the files in tree order; the diff panel lists files in this order so
  scrolling the diff and walking the tree agree.
- Viewed marks are keyed by file path and `fingerprint(old_blob, new_blob)`, not by review:
  a file counts as viewed while both sides stay byte-identical, so any new push that touches
  it un-views it (GitLab behaves the same), and the same change seen through a branch
  comparison or the GitLab MR shares one mark. Marks are stored per repository (its full
  path) in `app_db`.
- A file without its own mark still counts as viewed when marks chain from its old blob to
  its new one in two or more steps (`old → a → new`), of any path: the per-commit diffs of
  every commit that changed it cover the whole change, renames included. The one-step edge
  `old → new` of another path is left out, so an identical change in another file never
  vouches for this one. The search runs backward from the new blob (`into`), since the null
  blob has every added file as a successor. Unmarking such a file must remove the chain too
  (`covering_marks`), or it would stay viewed; it takes only steps between the two blobs,
  never one entering the old blob or leaving the new one, so a loop through the null blob
  (another file's deletion `x → 0`) is kept.
- `line_maps` walks hunks by their declared counts, so a removed line `--- x` or an added
  `+++ y` is never taken for a file header. `file_spans` cuts at every line starting with
  `diff --git `, which needs no hunk counts: a hunk body line always starts with a space,
  `+`, `-` or `\`. Both yield one entry per `diff --git` header, as gpui-component's
  `DiffFile::parse` yields one file; the caller checks that the `line_maps` and `file_spans`
  counts match and that each section parses to one file.
- `shown_block_starts` mirrors gpui-component's default folding (`visible_pairs` in its
  `diff/state.rs`, private): a change shows `context` unchanged lines on each side, a
  removal on the other side counts as a change between two lines of this side, folds
  shorter than `min_folded` stay open, and lines the patch lacks (between short-context
  hunks) never show. Folds the user opens are not known here.
- Blame chips (`marker_lines`): one on the first line of each run of lines from the same
  commit, plus one on every `shown_block_starts` line, so each block the diff shows starts
  with a chip even when its run began inside a fold. `initials`: the first letters of the
  first and last word (split at spaces, `.`, `_`, `-`; the local part of a bare e-mail),
  upper-cased, `?` when there is none. `tint_index` is FNV-1a of the id, so a commit keeps
  its color across files and launches.
- `language_override` answers only for paths gpui-component's `detected_language` misses
  (it already knows `Makefile`, `.proto`, `.cs`, `.java`, …), so the two never disagree;
  `None` keeps the component's detection. Names are gpui-component highlighter names; a
  language whose grammar is not compiled in renders as plain text. File names are matched
  case-insensitively on the last path component, `/` or `\` separated.
- `diff:<path>[#R120[-R135]]`, `L` for the old side, `R` for the new. `parse` normalizes a
  reversed range, takes `#R120-135` (second letter omitted) as the same side, and refuses
  everything else: line 0, mixed sides, a sign or other characters in a number, an empty
  path, a stray `%`, a path that is not UTF-8 once decoded. `format` writes a one-line range
  as `#R120` and encodes every byte of the path except ASCII letters, digits, `-._~` and
  `/`, so a path never breaks a markdown link target; `parse(format(x)) == x` for any path
  and any normalized range. `parse` also takes a raw (unencoded) path, for models that skip
  the encoding.
- `extract` follows CommonMark far enough not to invent links: it reads only the
  destination of an inline link whose `[` is not escaped (plain with balanced parentheses and
  backslash escapes, or `<…>`, which may hold spaces), skips fenced code blocks and inline code
  spans (a line at a time), ignores bare `diff:` text, autolinks and link labels, and drops
  `diff:` targets that do not parse. Repeats are kept because the order of mentions is the
  author's order. `locate` scans at most 4096 lines of a range, so a model's huge
  range cannot stall the UI thread; the file check (does the review contain the path) is the
  caller's.
- Fuzzy matching: the query's characters (whitespace dropped, case folded per char) must
  appear in order. An fzf-style DP picks the best-scoring alignment, not the greedy first
  one (approximately: the carried run bonus is not part of its state, as in fzf): each matched char scores, a char after `/` or `\` or at the start gets
  the biggest bonus, then after other separators (`-_.` and spaces), then camelCase humps
  and letter→digit; the first query char's bonus counts double; a run of consecutive
  matches carries its first char's bonus; gaps cost a start penalty plus one per skipped
  char. Positions are byte offsets so the UI can highlight with them directly.
- `rank`: an item matches on its label, or on its detail when it has one; a label match
  gets a fixed bonus so a name beats a path, and the better of the two is kept. Ties go to
  the shorter label, then to the given order, so an empty query keeps the caller's order.

- `check_svg` mirrors what usvg 0.46 (gpui's renderer, default options) would open, so
  nothing it lets through can make usvg read a file. svgz (gzip magic) is unpacked here,
  capped at `max_len`, and passed on unpacked, so usvg never unpacks it; a second gzip
  layer is not UTF-8 and is refused. The text is parsed with roxmltree 0.21 and
  `allow_dtd`, as usvg parses it, so hrefs and elements built by entities are seen. Every
  `href` (any namespace) of an `image` or `feImage` must be a data URL, parsed with usvg's
  `data-url` crate, whose type is neither `image/svg+xml` nor `text/plain` (both parse as
  a nested SVG, whose svgz usvg unpacks without a limit). An `feImage` `#id` is refused
  too: when usvg does not keep the target (a `<title>`, a foreign element) it reads `#id`
  as a file path relative to the working directory. usvg reads files through those two
  elements only: re-check that when gpui moves to another usvg.
- An embedded PNG, JPEG, GIF or WebP (the types usvg hands to resvg's decoders) whose
  header gives more than `max_pixels` is refused: resvg decodes it at full size whatever
  size it is drawn at, and zune-jpeg alone allows 16384² pixels. The size is read with
  imagesize 0.14, as usvg reads it; a picture whose size it cannot read usvg skips before
  decoding, so it passes.

## Decisions

- Viewed state is local only: GitLab keeps its "viewed" checkboxes in the browser and has
  no API for them.
