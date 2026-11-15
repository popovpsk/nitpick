# gitlab

Async client for the GitLab REST API v4: the current user, projects, merge requests with
their diff refs, every discussion of a merge request (everyone's comments, system notes
included), and writing diff comments, general comments, replies and resolve flags. Also
parses merge-request URLs and `!123` references. GPUI-free; plain async on tokio, awaited by
the app through `net_runtime::spawn`.

Not here: the host-neutral types the app uses (`forge`, which converts these models),
mapping diff lines to positions and hiding system notes (`review_core` / the app), finding
the GitLab instance from git remotes (`git_repo`), storing the token (`secret_store`).

## Files

- `lib.rs` — module list and the public re-exports.
- `client.rs` — `Client`: the reqwest client (from `http_client::builder`, plus auth, the
  total timeout and the same-host redirect policy), one method per endpoint (merge requests,
  discussions, notes: create, reply, edit, delete; resolve; approvals, approve, unapprove),
  discussion pagination.
- `request.rs` — what the client sends: `ProjectRef`, `MrQuery`/`MrState`, `NewPosition`
  and the JSON bodies, path-segment encoding.
- `models.rs` — what GitLab returns: `User`, `Project`, `MergeRequest` (with
  `user_notes_count`, reviewers and assignees), `DiffRefs`, `Discussion`, `Note`, `Position`,
  `LineRange`, `Approvals` / `Approver` (`user_can_approve` absent on old servers reads as
  allowed).
- `error.rs` — `Error` and the HTTP-status mapping.
- `mr_url.rs` — `parse_mr_url`, `parse_mr_url_with_base`, `parse_mr_reference`.
- `models_tests.rs` — JSON fixtures (shared with `client_tests.rs`) and deserialization.
- `client_tests.rs` — every endpoint against a `wiremock` server on the test's runtime.
- `mr_url_tests.rs` — URL and reference parsing tables.

## Spec and invariants

- Base URL: `scheme://host[:port][/relative-root]`; trailing slashes, a pasted `/api/v4`,
  query and fragment are dropped; API root is base + `/api/v4`. Credentials in the URL are
  rejected so the token stays the only secret.
- A project path is sent as one percent-encoded segment (`group%2Fsub%2Fproject`): GitLab
  reads `:id` as a single segment. Built with `percent-encoding` (everything but
  unreserved characters) and passed to `url` pre-encoded, which keeps `%2F` as is.
  Discussion ids are encoded the same way.
- Diff comment position (`NewPosition`, sent with `position_type: "text"`, absent lines
  omitted, not `null`): an added line sends only `new_line`, a removed line only `old_line`,
  an unchanged/context line sends both. Any other combination GitLab rejects or anchors
  wrongly. The three SHAs come from the merge request's `diff_refs`.
- `discussions` reads every page at `per_page=100`, following `x-next-page`, else a
  `Link: rel="next"` URL on the same scheme/host/port as the instance; it stops at 50 pages
  (5000 discussions), on an empty page, or at a missing next page, and warns on the cap.
- Errors: 401 → `Unauthorized`; 404 → `NotFound(<API path requested>)`; any other non-2xx →
  `Status` with GitLab's `message` (string as is, object/array as JSON text), else its
  `error`, else the raw body cut to 300 characters, else the status reason. Transport
  failures → `Http`; a 2xx body that does not parse → `Decode("<path>: <serde error>")`.
- Models ignore unknown fields and default missing or `null` scalars, so any GitLab version
  parses; only ids are required. `draft` falls back to the legacy `work_in_progress`;
  `source_project_id`/`target_project_id` fall back to `project_id`.
- `Discussion::is_resolved` is true when it has resolvable notes and all of them are
  resolved; `position()` is the first note's.
- The token travels only as the `PRIVATE-TOKEN` header, marked sensitive, never logged and
  hidden from `Debug`. Redirects are followed only to the same host (an `https` upgrade is
  allowed; `http_client::same_host_redirects`), because reqwest keeps custom headers across
  redirects. Requests are logged at debug level as method + API path.
- `parse_mr_url` treats everything between host and `/-/merge_requests/<iid>` as the
  project path (a relative root cannot be told apart from a group); `parse_mr_url_with_base`
  strips a known base (same host and port) first and falls back to `parse_mr_url` when the
  URL is not under it. Legacy URLs without `/-/` are not recognised.

## Decisions

Owner's calls; ask before changing.

- `reqwest` with the app's shared TLS setup from `http_client` (rustls on `ring`, OS
  certificate store, so a corporate CA works).
- Async only, on the app's shared tokio runtime: no blocking client, no runtime of its own,
  no threads.
- Personal/project access token in `PRIVATE-TOKEN`, not OAuth.
- Timeouts: whole request 60 s on top of `http_client`'s connect 15 s and read 60 s.
  User-Agent `nitpick`.
