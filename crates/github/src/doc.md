# github

Async client for the GitHub REST API v3 on github.com and GitHub Enterprise: the current
user, open pull requests, one pull request, review comments grouped into threads (with their
resolved state from GraphQL), conversation comments, and writing line comments, replies,
conversation comments and thread resolution. Also parses pull-request URLs and `#123`.
GPUI-free; plain async on tokio, awaited by the app through `net_runtime::spawn`.

Not here: review summaries (the text of an approve / request-changes review, `/reviews`),
the host-neutral types and the merge base of a pull request (`forge` and the app),
finding the repository from git remotes (the app), storing the token (`secret_store`).

## Files

- `lib.rs` — module list and the public re-exports.
- `client.rs` — `Client`: API roots from the web URL, the reqwest client (from
  `http_client::builder`, plus auth, the total timeout and the same-host redirect policy),
  one method per endpoint, Link-header pagination, the GraphQL calls.
- `request.rs` — what the client sends: `RepoRef` (`owner/name`), `Side`,
  `NewReviewComment`, the JSON bodies.
- `models.rs` — what GitHub returns: `User`, `PullRequest`, `BranchRef`, `ReviewComment`,
  `IssueComment`, `ThreadState`, `ReviewThread`, `group_threads`; the GraphQL shapes.
- `error.rs` — `Error` and the HTTP-status mapping.
- `pr_url.rs` — `parse_pr_url`, `parse_pr_url_with_base`, `parse_pr_reference`.
- `models_tests.rs` — JSON fixtures (shared with `client_tests.rs`), deserialization,
  thread grouping.
- `client_tests.rs` — every endpoint against a `wiremock` server on the test's runtime.
- `pr_url_tests.rs` — URL and reference parsing tables.

## Spec and invariants

- Base URL is the web address. `github.com`, `www.github.com` and `api.github.com` mean
  github.com: REST at `https://api.github.com`, GraphQL at `https://api.github.com/graphql`.
  Any other host is Enterprise: REST at `<base>/api/v3`, GraphQL at `<base>/api/graphql`; a
  pasted `/api/v3`, trailing slashes, query and fragment are dropped, credentials in the
  URL are rejected. Tests use the mock server as an Enterprise host.
- Every request sends `Authorization: Bearer <token>` (sensitive, never logged, hidden from
  `Debug`), `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28` and
  User-Agent `lgtm-ish`. Redirects are followed only to the API host
  (`http_client::same_host_redirects`), since reqwest keeps headers across redirects.
- Lists read `per_page=100` and follow `Link: rel="next"` only on the API's own
  scheme/host/port, stopping on an empty page, a missing link, or the cap: 3 pages of open
  pull requests (newest update first), 50 pages of review or conversation comments, 50
  GraphQL pages of review threads; a cap logs a warning.
- Threads: a review comment without `in_reply_to_id` starts a thread, a reply joins its
  root's (GitHub always points replies at the root; a reply whose root is gone becomes its
  own thread). Threads keep the root order of the API, comments inside root first, then by
  time. `review_threads` adds each thread's GraphQL state (node id, `isResolved`,
  `viewerCanResolve/Unresolve`) matched by the first comment's `databaseId`; when GraphQL
  fails (a token or Enterprise without it) the threads come back without state and a
  warning is logged, so comments still load.
- A line comment is `commit_id` + `path` + `line` + `side` (`RIGHT` for the new side,
  `LEFT` for the old); replies go to `/pulls/{n}/comments/{id}/replies` with the root's id;
  conversation comments live on the issue (`/issues/{n}/comments`). Resolution is the
  GraphQL `resolveReviewThread` / `unresolveReviewThread` mutation on the thread's node id.
- Errors: 401 → `Unauthorized`; 404 → `NotFound(<API path>)`; any other non-2xx → `Status`
  with GitHub's `message` plus each `errors[].message` (`"Validation Failed: …"`), else the
  raw body cut to 300 characters, else the status reason. A GraphQL answer with `errors` →
  `GraphQl` (messages joined). Transport failures → `Http`; an unparsable 2xx body →
  `Decode("<path>: <serde error>")`.
- Models ignore unknown fields and default missing or `null` values (a deleted user is
  `null`), so Enterprise versions parse; only ids and numbers are required.
- `parse_pr_url` reads `<owner>/<repo>/pull/<n>[/…]` on any host;
  `parse_pr_url_with_base` strips a known base (same host and port) first and falls back.
  `parse_pr_reference` takes `#123` or `123`.

## Decisions

Owner's calls; ask before changing.

- Same stack as `gitlab`: `reqwest` with `http_client`'s TLS, async on the app's runtime,
  timeouts 60 s total on top of `http_client`'s.
- Personal access token (classic or fine-grained) as a Bearer token, not OAuth or a GitHub
  App.
- Thread resolution through GraphQL, since REST has none; everything else is REST.
