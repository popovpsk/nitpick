# net_runtime

The app's one tokio runtime, started on first use and kept for the life of the process. Its
workers run the async network code of every other crate and of the app (`reqwest` in
`gitlab` and `updater`, `tokio::process` in `git_repo`); its blocking pool runs blocking
steps that belong to that work (the `updater`'s temp dir and `hdiutil`/`rsync`), so nothing
waiting on the network holds a thread of GPUI's executor. GPUI-free and knows nothing of
what runs on it.

Not here: the HTTP client setup (`http_client`); the network code itself (the crates that
call `spawn` and `unblock`).

## Files

- `lib.rs` — `handle`, `try_handle`, `spawn`, `unblock`, the runtime's shape.
- `tests.rs` — `unblock` awaited by a plain thread-parking executor (the way GPUI awaits
  it), `block_on` from the blocking pool, a panic reaching the awaiter.

## Rules

- `spawn(future)` runs async network work (`reqwest`, `tokio::process`) on the workers and
  returns a runtime-agnostic future of its output, awaited from GPUI tasks; a panic is
  re-raised in the awaiter, as with `unblock`.
- `unblock(work)` starts `work` on the blocking pool at once and returns a future of its
  result. The future is runtime-agnostic: GPUI tasks await it. Dropping the future does not
  stop the work, it only drops the result; cancellation stays the work's own (a stop flag,
  a timeout).
- A panic in `work` is re-raised in whoever awaits `unblock`, as if the work had run
  inline. A cancelled task cannot happen: the runtime is a static and never shuts down.
- `handle().spawn_blocking(work)` with the handle dropped is the fire-and-forget form.
- Blocking-pool threads may call `Handle::block_on`; worker threads and async code never
  may, and never run blocking calls — that would stall every async task on the runtime.
- Threads are named `net-runtime`, workers and blocking pool alike. The blocking pool grows
  on demand (tokio's default cap, 512) and drops a thread after 10 s idle.
- `try_handle` reports a runtime that failed to build; `handle` panics on it, like
  `std::thread::spawn` does when no thread can start.

## Decisions

Owner's calls; ask before changing.

- One runtime for the whole app, not one per crate.
- Two worker threads: the async load is light and blocking work goes to the pool.
