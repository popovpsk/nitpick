# agent

Runs the review assistant: starts an installed agent CLI (Claude Code, Codex, an ACP agent,
Pi) as a child process in the repository, feeds it prompts and turns its output into one
stream of `Event`s. This stage holds the shared machinery (process handling, line reading,
JSON-RPC framing, the session and registry) and the seam a harness driver plugs into; no
driver exists yet, so `Session::start` returns `Error::Unsupported` for every `HarnessKind`.
GPUI-free.

Not here: UI, settings and persistence (`nitpick`, `app_db`), the prompt and brief texts, the
`diff:` link format (`review_core`), the per-harness wire formats (the drivers, when they
land).

## Files

- `lib.rs` — the contract: `HarnessKind`, `AcpFlavor`, `Launch`, `Event`, `Outcome`,
  `Failure`, `FailureKind`, `Error`, `probe`; re-exports `Session` and `shutdown_all`. The app
  uses only these.
- `process.rs` — command splitting, login-shell environment snapshot, environment merge and
  scrub, program lookup, `Invocation`, `Process` (spawn in a process group / job object, soft
  shutdown, stderr tail), `probe_version`, `on_runtime`.
- `lines.rs` — `Lines`: newline framing over any `AsyncRead` that survives bad UTF-8 and
  skips lines over 32 MiB.
- `jsonrpc.rs` — line-framed JSON-RPC 2.0 over an `AsyncRead` + `AsyncWrite`: `connect`,
  `Client` (requests, notifications, replies to server requests), `Connection` (incoming
  notifications and server requests).
- `registry.rs` — `Registry` of live sessions and `shutdown_all`.
- `session.rs` — `Session`, and the driver seam: `Driver`, `Plan`, `Transport`, `Channels`,
  `Command`, `driver_for`, `start_in`, the supervisor task.
- `tests/` — unit tests per module (`#[cfg(unix)]` for the ones that spawn `sh`).

## The driver seam

A driver implements `session::Driver`: `plan(&Launch)` returns the arguments and environment
it adds after the user's command (and writes its files into `Launch.scratch`, which the
session creates); `run(Launch, Transport, Channels)` is the whole conversation, a future that
reads the harness's stdout and writes its stdin through the boxed `Transport` and returns when
the session is over. `Channels` carries the `flume::Receiver<Command>` (prompt, interrupt,
shutdown, in call order) and the `flume::Sender<Event>`. `driver_for` is the only place that
maps a `HarnessKind` to a driver. Tests run a driver on `tokio::io::duplex` halves
without a process; `start_in` takes a registry and an `Environment` so tests start real
sessions without the login shell.

## Spec

- All process work runs on `net_runtime`'s tokio runtime through `handle().spawn(..)` with
  the `JoinHandle` kept (`net_runtime::spawn` would detach a task whose future is dropped).
  Futures handed to the app (`shutdown`, `shutdown_all`, `probe`) are runtime-agnostic: the
  work is spawned when they are created and awaiting only joins it.
- The supervisor ends a session when the driver returns or the process exits: on an exit it
  gives the driver one second to read the rest of stdout. Then it stops the process and sends
  `Event::Exited` exactly once, last (`None` after a requested shutdown, otherwise the stderr
  tail, else the exit status); drivers never emit `Exited`. A panicking driver still unregisters
  the session and wakes whoever waits for it.
- A driver must return on `Command::Shutdown` and on end of stdout, dropping the transport:
  that closes stdin, which is step one of the soft shutdown.
- Soft shutdown: stdin closed, wait 3 s, SIGTERM to the process group, wait 2 s, SIGKILL to
  the group. On Windows there is no SIGTERM step: after the stdin wait the job object is
  terminated. A killed Claude Code can lose the user's login if it is mid token refresh, hence
  the order. After the leader is gone the group is killed once more, so a background command the
  agent started never outlives the session; dropping a `Process` does the same.
- Dropping the last `Session` clone requests the same shutdown; the registry keeps only the
  control handle, so it never keeps a session alive. `shutdown()` aborts the supervisor if the
  session has not ended in 10 s (the drop of `Process` then kills the group).
- `Session::start` is synchronous (it must report "not found" as an error) and so waits for the
  login-shell snapshot the first time; `process::prewarm()` starts it in the background, the
  app calls it at startup.
- The login-shell snapshot is `$SHELL -lic` printing `env` between markers, 5 s limit, cached
  for the process (a failure caches an empty snapshot and the app's own environment is used).
  `$SHELL` is accepted only as an absolute path to a sh-like shell, else `/bin/sh`. It is not
  captured on Windows, where the process environment is already the user's.
- Environment of a child: the app's variables, plus the shell's for names the app lacks
  (not `PWD`, `OLDPWD`, `SHLVL`, `_`), minus the scrubbed set (Claude Code nesting markers,
  `ORCA_*`, the `GIT_*` variables `git_repo` also scrubs), then the driver's variables. `PATH`
  is shell entries, then the app's, then the known install folders (`~/.local/bin`,
  `~/.claude/local`, `~/.opencode/bin`, `~/.codex/bin`, `~/.npm-global/bin`,
  `/opt/homebrew/bin`, `/usr/local/bin`), without duplicates. Provider keys are left alone:
  they are the user's choice.
- Program lookup walks that `PATH` itself (executable bit on Unix; `PATHEXT` on Windows, only
  for names without an extension). A name with a separator is used as given. Windows gets
  `CREATE_NO_WINDOW`; JSON and long texts never go in arguments there (`.cmd` wrappers break
  quoting), only paths of files in `scratch`.
- stderr is read in parallel and the last 4 KiB kept; a read error ends only that reader.
- Parsing is tolerant: a non-JSON line is logged at debug and skipped, invalid UTF-8 becomes
  U+FFFD, an over-long line is dropped, unknown messages are ignored. One bad line never ends a
  connection; only end of stream or a failed write does, and then pending requests fail with
  `CallError::Closed`.
- `jsonrpc::Options::tag_version = false` leaves the `"jsonrpc":"2.0"` field out for peers
  (Codex) that omit it. Incoming request ids may be numbers or strings; ours are numbers.
  `Connection::close` flushes what was queued and ends the writer; dropping it aborts both tasks.
- `probe` runs `<command> --version` with a 10 s limit in the temp folder and returns the first
  non-empty line (stdout, else stderr).

## Decisions

- `process-wrap` (tokio) for the process group and the job object instead of `nix` and
  `taskkill`: no `unsafe` in the crate and one code path for both platforms; the soft shutdown
  signals through its `signal`.
- Modules are `pub` so drivers and tests reach them; the app is expected to use only the
  contract in `lib.rs`.
- `Error::Unsupported` is a unit variant.
- No `futures` crate: the shutdown futures join `JoinHandle`s and a `watch` channel.
- Drivers sit on `AsyncRead` + `AsyncWrite`, not on `Child`, so recorded sessions replay over
  `duplex` the same on every OS.
