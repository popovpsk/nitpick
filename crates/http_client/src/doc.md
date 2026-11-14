# http_client

The one place the app's `reqwest` clients are configured: rustls on the `ring` provider with
the OS certificate store, the shared timeouts, and the redirect policies. `builder(user_agent)`
returns a `reqwest::ClientBuilder` that callers finish (headers, total timeout, redirect policy)
and build. GPUI-free; the clients run on `net_runtime`.

Not here: the requests themselves (`gitlab`, `updater`); the tokio runtime (`net_runtime`).

## Files

- `lib.rs` — `builder`, `same_host_redirects`, `redirect_allowed`, `Error`, the timeout and
  redirect constants, the rustls config.
- `tests.rs` — the client builds with the shared TLS setup; the redirect rule table.

## Spec and invariants

- TLS: a preconfigured `rustls::ClientConfig` on the `ring` provider, safe default protocol
  versions (TLS 1.2 and 1.3), `rustls-platform-verifier`, ALPN `h2` then `http/1.1`. Passed
  with `tls_backend_preconfigured`; reqwest is built with `rustls-no-provider`, so nothing
  installs a process-wide crypto provider and no other TLS stack is linked.
- `ring`, not `aws-lc-rs`: no C/CMake build of aws-lc on every target (Windows arm64 and
  offline package builds included), and `ring` is enough for a client.
- Platform verifier, not bundled `webpki-roots`: certificates are checked by the OS store, so
  a corporate or self-hosted GitLab CA the user trusts works, and revocation follows the OS.
- `builder` sets connect 15 s and read 60 s (per read, re-armed after each one: the stall
  detector, so long downloads of a healthy file are not cut). It sets no total timeout; each
  caller picks its own on the client or per request.
- Redirects: `builder` follows up to 10 to anywhere, for callers that send no secrets
  (`updater` adds `https_only`, GitHub release assets redirect to a CDN host). A caller that
  sends a token installs `same_host_redirects(origin)`: reqwest keeps custom headers across
  redirects, so it follows only the origin's host (any port, same scheme or an upgrade to
  `https`; never a downgrade), stops (returns the 3xx) on any other target, errors after 10.
  `redirect_allowed` is that rule as a pure function.

## Decisions

Owner's calls; ask before changing.

- `reqwest` on rustls with `ring` and `rustls-platform-verifier`: no OpenSSL/native-tls, no
  `aws-lc`.
- One shared builder for every HTTP client in the app, so the TLS stack is chosen once.
