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
  `web_url`, `new_token_url`: GitLab's `/-/user_settings/personal_access_tokens?name=&scopes=api`,
  GitHub's `/settings/tokens/new?description=&scopes=repo`), `User` (with `avatar_url`), `Request` (`description`: GitLab's description or GitHub's body, `None` when blank; reviewers, assignees, `comments` — GitLab's
  `user_notes_count`, `None` on GitHub, whose list has no count — and `updated_at`), `Approval`,
  `RequestState`, `DiffRefs`, `Discussion`, `Note`, `Position`, `NewPosition`,
  `GITHUB_DEFAULT_URL`; the pending review: `PendingReview` (`handle`: GitHub's review id and
  node id, `None` on GitLab, whose drafts belong to no review object), `Draft` (`id`: GitLab's
  draft note id or GitHub's comment node id; a reply has `reply_to` and no position),
  `DraftTarget`, `Verdict`, and per host `general_drafts`, `resolves_on_submit`, `verdicts`.
- `client.rs` — `Client`: `new(kind, base_url, token)`, `kind`, `base_url`, and the
  operations `current_user`, `open_requests`, `request`, `discussions`, `comment`, `reply`,
  `set_resolved`, `edit_note`, `delete_note`, `approval`, `approve`, `unapprove`; the review:
  `pending_review`, `add_draft`, `edit_draft`, `delete_draft`, `submit_review`,
  `discard_review`.
- `convert.rs` — each host's models to the neutral ones and back (positions, comment
  payloads, discussion ids); `own_avatar` keeps a user's `avatar_url` only when it is not a
  Gravatar link (gravatar.com or a subdomain), which GitLab hands out for users without an
  uploaded picture — those show initials and no request leaves for Gravatar.
- `error.rs` — `Error`: the host's error as is (`transparent`), plus the neutral refusals
  (`Unsupported`: withdrawing a GitHub approval; `DraftLineIgnored`: a GitLab line draft
  stored without its line).
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

- Editing or deleting a note on GitHub picks the endpoint by the discussion id: `issue-<id>`
  is an issue (conversation) comment, anything else a review comment.
- Approval: GitLab answers it (`approvals`: who approved, `user_has_approved`,
  `user_can_approve`). On GitHub it is derived from the reviews: each reviewer's latest
  `APPROVED` / `CHANGES_REQUESTED` / `DISMISSED` verdict counts (comments don't reset it), and
  the current user may approve unless they already did or wrote the pull request. A GitHub
  approval can't be withdrawn (dismissing a review needs maintainer rights), so
  `can_unapprove` is always false there. Neither is offered on a request that is not open;
  `approval` takes the already known username so GitHub needs no `/user` call each time.

- Review drafts live on the host (GitLab draft notes, GitHub's pending review), visible only
  to their author until submitted, so they survive restarts and match the web UI. Submitting
  on GitLab is `bulk_publish`, then the approval (skipped when `approvals` says it is
  already given), then the summary as a general comment, last: GitLab has no review object
  to carry either, so a submit can stop halfway, and this order lets a retry repeat nothing.
  GitHub takes no general draft (its review summary is the review body) and has no
  resolve-on-submit. A GitHub draft without a known pending review looks one up first (it
  may have been started on the web or by a draft whose reload has not landed: one pending
  review per user and request) and starts one only when there is none, at the draft's
  `head_sha` (`commitOID`), so its line numbers mean the loaded diff even after a push.
- GitLab drafts and their line: GitLab before 16.3 ignores a draft's `position`, so a line
  draft whose answer has no line (`DraftNote::on_line`) is deleted again and refused with
  `DraftLineIgnored` rather than left to publish as a general comment. Until GitLab fixed
  it, its draft update replaced the position with whatever the request carried, so a
  text-only edit turned a line draft into a general one; `edit_draft` takes the `Draft` and,
  when the answer has lost the line, sends the edit again with the position rebuilt from
  it (`gitlab_line_position`: no line range, so on those versions a multi-line draft made on
  the web comes back as a draft on its last line). A server that keeps the position takes
  one call.

## Decisions

Owner's calls; ask before changing.

- An enum, not a trait object: async methods stay plain `async fn` on tokio, no boxing.
- A separate crate rather than an app module, so the conversions are tested without GPUI
  and the app depends on one neutral crate instead of every host crate.
- GitHub thread resolution through GraphQL; where it cannot work the button is hidden
  rather than failing on click.
