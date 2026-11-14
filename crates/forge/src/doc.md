# forge

The host-neutral layer between the app and the places a merge request comes from: GitLab
merge requests and GitHub pull requests are both a `Request` with `Discussion`s, and
`Client` is an enum over the `gitlab` and `github` clients whose async methods dispatch with
a `match`. The app (`review/`, `workspace/start.rs`, settings) uses only this crate's types;
it names a concrete host only through `ForgeKind`. GPUI-free.

Not here: the HTTP clients and their wire models (`gitlab`, `github`), the merge base of a
request that has none and the remote to fetch from (the app's `review::model`), settings and
tokens (the app, `secret_store`).

## Files

- `lib.rs` — module list and the public re-exports.
- `models.rs` — `ForgeKind` (reference `!5` / `#5`, refspecs, `comment_context`,
  `web_url`), `User`, `Request`,
  `RequestState`, `DiffRefs`, `Discussion`, `Note`, `Position`, `NewPosition`,
  `GITHUB_DEFAULT_URL`.
- `client.rs` — `Client`: `new(kind, base_url, token)`, `kind`, `base_url`, and the
  operations `current_user`, `open_requests`, `request`, `discussions`, `comment`, `reply`,
  `set_resolved`.
- `convert.rs` — each host's models to the neutral ones and back (positions, comment
  payloads, discussion ids).
- `error.rs` — `Error`: the host's error as is (`transparent`), plus the neutral refusals.
- `request_url.rs` — `parse_request_url` (GitLab or GitHub URL → kind, project, number),
  `parse_reference` (`!123`, `#123`, `123`).
- `convert_tests.rs`, `client_tests.rs`, `request_url_tests.rs` — conversions, dispatch to
  each host's endpoints on a `wiremock` server, URL tables.

## Spec and invariants

- `ForgeKind` serializes as `"gitlab"` / `"github"` and defaults to GitLab, so stored data
  written before GitHub existed reads as GitLab. A new host is a new variant, a new client
  crate and a new `Client` arm; the app adds only its settings fields and host names.
- `DiffRefs`: `head_sha`, the target tip `start_sha`, and `base_sha` only when the host
  computes the merge base (GitLab). GitHub's `base.sha` is the target tip, and its PR diff is
  three-dot, so `base_sha` is `None` and the app takes `git merge-base start head` after
  fetching; `needed_commits` names what must exist locally for that (head + merge base, or
  head + target tip). GitHub's compare endpoint (`merge_base_commit`) is not used: its answer
  carries every changed file's patch.
- `Request::refspecs`: GitLab `refs/merge-requests/<n>/head` → `refs/nitpick/mr/<n>`
  (unchanged from before GitHub); GitHub `refs/pull/<n>/head` → `refs/nitpick/pr/<n>/head`
  plus the target tip `start_sha` itself → `refs/nitpick/pr/<n>/base` (GitHub serves
  reachable commits by id), so the merge base is computed from exactly the commit GitHub
  reported, and a deleted or force-pushed target branch does not break the fetch. Names
  under one prefix never nest, since a ref cannot be both a file and a folder.
- `comment_context`: GitHub accepts a line comment only inside its diff hunks (3 lines of
  context), so the app offers a line only within that distance of a change; GitLab takes
  any line. `web_url` normalises a configured address: GitHub's empty, `www.` or `api.`
  github.com is `https://github.com`, so remotes on github.com match.
- `Discussion.id` is what `reply` takes: GitLab's discussion id; on GitHub the thread root's
  comment id, or `issue-<id>` for a conversation comment, which has `can_reply: false`
  (`reply` refuses it with `CannotReply`), as has a thread whose root comment is gone (its
  first comment is a reply, and GitHub takes replies only to a root). `resolve_id` is what `set_resolved` takes, `None`
  when the viewer cannot toggle it: GitLab's discussion id for a resolvable discussion; on
  GitHub the review thread's GraphQL node id when GraphQL answered and allows the toggle in
  that direction. `resolved` is shown either way.
- GitHub positions: a current comment (`line` set) is at `commit_id`, an outdated one at
  `original_commit_id` / `original_line`, so the app's "same head" check sends outdated ones
  to the sheet. `side` `LEFT` is an old line, `RIGHT` a new one; both paths are the comment's
  `path`. A new line comment is `RIGHT` + `new_line` when there is one (added or context
  line), else `LEFT` + `old_line`, on `head_sha` and the new path; neither line →
  `InvalidPosition`. GitLab positions pass through unchanged.
- GitHub `discussions` = review threads + conversation comments, ordered by their first
  note's time. Notes keep `system`; only GitLab sets it.
- `parse_request_url` tries GitLab's `/-/merge_requests/<n>` shape (under the GitLab base,
  else any host), then GitHub's `/<owner>/<repo>/pull/<n>`; the two never overlap. The host
  of the URL is not checked against settings: the configured client for that kind opens it.

## Decisions

Owner's calls; ask before changing.

- An enum, not a trait object: async methods stay plain `async fn` on tokio, no boxing.
- A separate crate rather than an app module, so the conversions are tested without GPUI
  and the app depends on one neutral crate instead of every host crate.
- GitHub thread resolution through GraphQL; where it cannot work the button is hidden
  rather than failing on click.
