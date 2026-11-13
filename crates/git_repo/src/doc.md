# git_repo

Everything the app needs from git, through the `git` CLI: opening a work tree, branches and
remotes, the merge-base comparison a merge request shows, the changed-file list and the
patch, the commits of a range, and the network operations (`fetch`, the per-branch `pull`).
Local operations are blocking and never touch the network; network operations are `async`
on `tokio::process` and run on `net_runtime`. GPUI-free.

Not here: patch parsing into hunks and the file tree (`review_core`), GitLab API calls
(`gitlab`), deciding which branches to show or when to pull (the app).

## Files

- `lib.rs` — `GitError` and the plain data types (`Branch`, `Comparison`, `DiffOptions`,
  `ChangedFile`, `Commit`, `Remote`, `PullOutcome`).
- `command.rs` — building and running every `git` process, blocking and async, with the
  fixed config/env; NUL-record splitting; `NETWORK_TIMEOUT`.
- `repo.rs` — `Repo`: open, branches, current branch, default target, resolve, `compare`,
  commits, remotes, upstream remote.
- `diff.rs` — `changed_files` (one `--raw --numstat -z` run) and `diff` (the patch text).
- `pull.rs` — `fetch` and the fast-forward-only `pull`.
- `remote_url.rs` — `parse_remote_url`: host, project path and web base of a remote URL.
- `tests.rs` — throwaway repositories in a `TempDir` with the real `git` and an empty
  global config; bare "origin" plus a second clone for the remote and pull cases.

## Invariants

- Every invocation is `git -C <root> -c core.quotepath=false -c color.ui=false …` with
  `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, stdin null, stdout and stderr captured, and the
  `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`-style variables removed so an inherited
  environment cannot redirect it to another repository. On Windows no console window opens.
  A failure carries git's stderr in `GitError::Command`.
- Revisions, remotes and refspecs starting with `-` (or empty) are rejected before git
  runs, so a name can never become an option.
- Async commands use `kill_on_drop` and the repo's network timeout (`NETWORK_TIMEOUT`,
  120 s, overridable by `with_network_timeout`); on expiry the child is killed and
  `TimedOut` returned. Dropping the future kills the child too.
- One git process per operation where possible; never per file.
- Paths come from `-z` output, so spaces, unicode, tabs and newlines round-trip; non-UTF-8
  bytes are replaced lossily.
- Patches and file lists ignore the user's diff config that changes their shape:
  `diff.noprefix`, `diff.mnemonicPrefix`, `diff.relative` are forced off; `--no-color
  --no-ext-diff --no-textconv`, `a/`/`b/` prefixes, full blob ids. `diff.algorithm` is
  left to the user.
- Blob ids are full length; the absent side of an add or delete is all zeros. Additions and
  deletions are `None` for binary files. `path` is the new path (the old one for a delete);
  `old_path` is set only for renames and copies.
- `current_branch` is `None` for a detached or unborn HEAD.
- `branches`: local first, then remote-tracking, each newest committer date first, then by
  name; symbolic refs such as `origin/HEAD` are left out. `upstream` is the short name.
- `default_target`: the remote's HEAD (`origin` when any `origin/…` branch exists, else the
  upstream remote) as the local branch of that name if it exists, else as the remote ref;
  otherwise the first of `main`, `master`, `develop`, `trunk`, each tried local then remote.

## Comparison rule

`compare(target, source)` shows what the source introduces: the diff from a merge base to
the source head. The merge base is `git merge-base <source> <target> <extras…>`, where the
extras are the target's upstream when the target is a local branch, or every local branch
tracking the target when the target is a remote-tracking ref. With three or more commits
git takes the merge base of the source and a hypothetical merge of the rest, so a stale
local target whose upstream the source has already merged does not drag upstream commits
into the diff. An extra that is the source ref itself is skipped, so comparing `main`
against `origin/main` still shows what upstream brings. `merge_base_refs` lists the refs
used. No common history is `NoMergeBase`.

## Pull rule

`pull(branch)` is fast-forward only and touches nothing but that branch:

- A local branch wins over a remote-tracking ref of the same name, as in git's ref lookup.
- Remote-tracking `remote/rest` (longest matching remote name): fetch
  `+refs/heads/rest:refs/remotes/remote/rest`; `Updated` or `UpToDate` by comparing the ref
  before and after (`from` is all zeros when the ref did not exist).
- Local branch with an upstream: fetch the upstream into its tracking ref (skipped for a
  `.` upstream). Equal, or local ahead of upstream: `UpToDate`. Diverged: `NotFastForward`.
  Otherwise, if the branch is checked out in this work tree, `merge --ff-only
  --no-autostash` (git refuses when local changes would be overwritten; the error carries
  its message); if it is checked out in another work tree, `CheckedOutElsewhere`; else
  `update-ref <branch> <new> <old>`, a compare-and-swap.
- Local branch without an upstream, or whose upstream does not map into `refs/remotes/`:
  `NoUpstream`. Neither local nor matching a remote: `UnknownBranch`.

## Decisions

Owner's calls; ask before changing.

- The `git` CLI, not libgit2/gix: same behaviour as the user's git (credentials, config,
  hooks, new repository formats) and no native build dependency.
- Pull is fast-forward only; the app never merges, rebases or rewrites history.
- The merge base is taken against the target and its upstream (or its tracking branches),
  not the target alone.
