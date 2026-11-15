# avatar_cache

Fetches the avatar pictures of comment authors and keeps them: a download that sends the
forge token only to the forge's own origin, a normalizer that turns any picture into a 64 px
square PNG, and a cache keyed by the avatar URL in `app_db`. GPUI-free; the app runs `fetch`
on `net_runtime` and `normalize` on its background executor.

Not here: which URLs are wanted and the in-memory URL → image map the UI reads
(`nitpick` `review/avatars.rs`), dropping Gravatar links (`forge`'s conversion).

## Files

- `lib.rs` — `Auth` (`gitlab` / `github`: the token header for one origin, `applies_to`),
  `Downloader::fetch`, `normalize`, `Cache` (`read` with a freshness flag, `write`), `Error`.
- `tests.rs` — normalizing, refusals, cache keys and age, token scoping and redirects on
  `wiremock` servers.

## Spec

- The token goes only to a URL whose origin (scheme, host, port) equals the forge's:
  requests with it do not follow redirects by themselves; each hop is checked again, so a
  redirect to object storage or a CDN is fetched without the token. Requests without it
  follow redirects as usual. At most five hops.
- A body over `MAX_BYTES` (2 MB) is refused, by `Content-Length` and while streaming. A
  picture over `MAX_PIXELS` is refused from its header before decoding, so a small file with
  huge dimensions never allocates the bitmap. The picture is cropped to its centred square
  before scaling, so a 1×N strip never scales its long side up into a huge buffer. Animated
  pictures keep their first frame.
- `Cache` reads and writes `app_db`'s `avatars` table. An entry older than `MAX_AGE` (7 days)
  is stale: the app shows it and refetches, and keeps it when the refetch fails. The database
  drops entries older than 90 days at startup.

## Decisions

- The cache key is the URL alone: GitLab gives a new upload path when a user changes their
  picture; GitHub keeps the URL, which the week-long freshness covers.
- Pictures are normalized once on download, so the UI decodes small PNGs only.
