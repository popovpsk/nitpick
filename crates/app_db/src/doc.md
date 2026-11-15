# app_db

The app's SQLite database (`nitpick.db` in the data folder) and the only crate that touches
it: viewed marks per repository and the avatar cache. One thread owns the connection; every
call queues a job on it and returns a future for the answer, so callers on any thread never
block and jobs run in the order they were called. GPUI-free.

Not here: settings and the recent list (JSON, `nitpick`'s `settings_store.rs` /
`recent.rs`), the in-memory viewed map (`review_core::ViewedStore`), when marks are loaded or
written (`nitpick` `review/viewed.rs`), fetching avatars (`avatar_cache`).

## Files

- `lib.rs` — `Db` (`open`, `open_in_memory`; `load_viewed`, `set_viewed`, `avatar`,
  `put_avatar`, `prune_avatars`, `barrier`), `ViewedMark`, `Error`, the migrations.
- `tests.rs` — marks per repository, call order, expiry, avatars, reopening, a newer schema,
  two openers creating the file at once.

## Spec

- A call sends its job when it is made, not when its future is first polled: the order of
  calls is the order of execution, so a mark followed by an unmark always ends unmarked
  even if nobody awaits the first. `barrier` resolves once every earlier job ran; the app
  awaits it at quit.
- The thread lives while any `Db` clone does; a dropped clone does not stop it. A job that
  panics is caught and logged, so one bad job never stops the queue.
- WAL, `synchronous = NORMAL` (no fsync per commit), a 5 s busy timeout so a second app
  process waits instead of failing; the timeout is set before switching to WAL, and the
  switch is retried while busy (it doesn't use the busy handler), so two processes creating
  the file at once both open it. A multi-mark write is one transaction.
- Schema: `MIGRATIONS[i]` takes `user_version` from `i` to `i + 1`, each in an immediate
  transaction that re-reads the version inside it, so a racing process never applies a step
  twice;
  a database newer than the binary is refused (`TooNew`) and the app falls back to an
  in-memory one for the session. Change the schema only by appending a migration.
- `viewed` (repo, path, fingerprint → viewed_at): `load_viewed` first deletes that
  repository's marks older than `VIEWED_RETENTION_SECS` (180 days). `avatars` (url → png,
  fetched_at): `prune_avatars` drops entries older than `AVATAR_RETENTION_SECS` (90 days).

## Decisions

- Data in SQLite, settings in JSON, as in pawse: viewed marks grow with every review, and
  rewriting one JSON per repository on each mark doesn't scale to tens of repositories with
  merge requests of hundreds of files.
- One owning thread instead of a shared connection behind a mutex: writes are serialized by
  construction and the UI never waits on a lock.
- The old `reviews/*.json` files are not imported; marks start empty with this database.
