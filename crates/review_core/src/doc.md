# review_core

The review model the UI draws: the changed-file tree with folder compression and viewed
roll-up, the per-repository store of "viewed" marks, and the map from a diff line to the
old/new line pair GitLab positions need. GPUI-free and git-free; only the store does file
I/O.

Not here: producing the file list or the patch (`git_repo`), talking to GitLab (`gitlab`),
drawing the tree or deciding when to save (`slopcop/src/review/doc.md`).

## Files

- `tree.rs` — `FileTree::build` from `TreeFile`s, `rows` (the visible rows for a set of
  collapsed folder paths), `progress` (viewed/total per node), `file_order`, `ancestors`.
- `viewed.rs` — `ViewedStore` (load, `is_viewed`, `set_viewed`, `snapshot` / `save`),
  `fingerprint`, `write_atomic`.
- `line_map.rs` — `line_maps(patch)`: one `LineMap` per `diff --git` file in patch order;
  `LineMap::locate(side, line)` → `LineKind` (context with both numbers, added, removed).
- `tests.rs` — tree shape and order, collapse, roll-up, store round trip and expiry,
  line-map cases including a removed line that looks like a `---` header.

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

## Decisions

- Viewed state is local only: GitLab keeps its "viewed" checkboxes in the browser and has
  no API for them.
