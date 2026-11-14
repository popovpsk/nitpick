# review_core

The review model the UI draws: the changed-file tree with folder compression and viewed
roll-up, the per-repository store of "viewed" marks, the map from a diff line to the
old/new line pair GitLab positions need, the fuzzy matcher every picker filters with, and
the syntax language of paths the diff component does not recognise. GPUI-free and git-free;
only the store does file I/O.

Not here: producing the file list or the patch (`git_repo`), talking to GitLab (`gitlab`),
drawing the tree or deciding when to save (`lgtm-ish/src/review/doc.md`), the picker UI
(`lgtm-ish/src/picker.rs`).

## Files

- `tree.rs` — `FileTree::build` from `TreeFile`s (`build_with` leaves out files a filter
  rejects and keeps the others' indices; `build_shaped` with a `TreeShape` builds a flat
  list sorted by path or a tree without folder compression; `progress_tracked` counts only
  the files a filter tracks), `rows` (the visible rows for a set of
  collapsed folder paths), `progress` (viewed/total per node), `file_order`, `ancestors`.
- `viewed.rs` — `ViewedStore` (load, `is_viewed`, `set_viewed`, `snapshot` / `save`),
  `fingerprint`, `write_atomic`.
- `line_map.rs` — `line_maps(patch)`: one `LineMap` per `diff --git` file in patch order;
  `LineMap::locate(side, line)` → `LineKind` (context with both numbers, added, removed);
  `LineMap::changes()` → each run of changed lines (`Change`: its first line, and an anchor
  up to two context lines above it within the hunk, else the first line).
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
  comparison or the GitLab MR shares one mark. One JSON per repository, named after the
  folder plus a hash of its full path. Marks older than 180 days are dropped on load. An
  unreadable file starts empty rather than failing the review.
- `line_maps` walks hunks by their declared counts, so a removed line `--- x` or an added
  `+++ y` is never taken for a file header. The result is index-aligned with
  gpui-component's `DiffFile::parse` of the same patch, which also yields one file per
  `diff --git` header; the caller checks the counts match.
- `language_override` answers only for paths gpui-component's `detected_language` misses
  (it already knows `Makefile`, `.proto`, `.cs`, `.java`, …), so the two never disagree;
  `None` keeps the component's detection. Names are gpui-component highlighter names; a
  language whose grammar is not compiled in renders as plain text. File names are matched
  case-insensitively on the last path component, `/` or `\` separated.

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

## Decisions

- Viewed state is local only: GitLab keeps its "viewed" checkboxes in the browser and has
  no API for them.
