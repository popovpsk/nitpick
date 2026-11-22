# agent

Runs the review assistant: starts an installed agent CLI (Claude Code, Codex, an ACP agent,
Pi) as a child process in the repository, feeds it prompts and turns its output into one
stream of `Event`s. It holds the shared machinery (process handling, line reading,
JSON-RPC framing, the session and registry), the seam a harness driver plugs into and the
drivers: Claude Code, Codex, the ACP agents (OpenCode, DeepSeek, any other) and Pi. GPUI-free.

It also holds the texts the app sends: the system instructions, the tasks and the review brief
(`prompt`, `brief`).

Not here: UI, settings and persistence (`nitpick`, `app_db`), filling a `Brief` from review data
(`nitpick`), the `diff:` link format (`review_core`, which `prompt` calls).

## Files

- `lib.rs` — the contract: `HarnessKind`, `AcpFlavor`, `Launch`, `Event`, `Outcome`,
  `Failure`, `FailureKind`, `Error`, `probe`; re-exports `Session` and `shutdown_all`. The app
  uses only these.
- `process.rs` — command splitting, login-shell environment snapshot, environment merge and
  scrub, program lookup, `Invocation`, `Process` (spawn in a process group / job object, soft
  shutdown, stderr tail), `drain`, `probe_version`, `on_runtime`.
- `lines.rs` — `Lines`: newline framing over any `AsyncRead` that survives bad UTF-8 and
  skips lines over 32 MiB.
- `jsonrpc.rs` — line-framed JSON-RPC 2.0 over an `AsyncRead` + `AsyncWrite`: `connect`,
  `Client` (requests, notifications, replies to server requests), `Connection` (incoming
  notifications and server requests).
- `registry.rs` — `Registry` of live sessions and `shutdown_all`.
- `session.rs` — `Session`, and the driver seam: `Driver`, `Plan`, `Transport`, `Channels`,
  `Command`, `driver_for`, `start_in`, the supervisor task.
- `prompt.rs` — system instructions (`instructions(language, disk_matches_head)`), the overview
  and update tasks, `with_brief`, `with_new_comments`, `selection_link`, `selection_question`
  and `Selection`.
- `brief.rs` — `Brief` and its parts (plain data), `render`, `render_threads`, thread digest and
  fingerprint (`Thread::digest`, `Thread::fingerprint`, `fingerprints`, `changed_since`), the limits.
- `acp/` — the generic ACP driver and its flavors (`mod.rs`, `wire.rs`, `normalize.rs`,
  `permission.rs`, `flavor.rs`); see `acp/doc.md`. OpenCode is verified live, DeepSeek is not.
- `claude/` — the Claude Code driver (`mod.rs`, `wire.rs`, `normalize.rs`); see `claude/doc.md`.
- `codex/` — the Codex driver (`mod.rs`, `wire.rs`, `normalize.rs`, `classify.rs`); see
  `codex/doc.md`. Written from documentation, not run against a real CLI.
- `pi/` — the Pi driver (`mod.rs`, `wire.rs`, `normalize.rs`, `classify.rs`); see `pi/doc.md`.
  Written from documentation, not run against a real CLI.
- `fixtures/` — recorded harness sessions (`claude/*.jsonl` stdout, `opencode/*.jsonl` with the
  `.sent.jsonl` requests) that drive the driver tests; they are never edited by hand. The Codex
  and Pi tests use synthetic transcripts written in the test code, not fixtures; the ACP tests replay
  the OpenCode recordings and add scripted peers.
- `tests/` — unit tests per module (`#[cfg(unix)]` for the ones that spawn `sh`).

## The driver seam

A driver implements `session::Driver`: `plan(&Launch)` returns the arguments and environment
it adds after the user's command (and writes its files into `Launch.scratch`, which the
session creates; it runs synchronously inside `Session::start`, so it may block for a bounded
time, as the OpenCode flavor's version probe does); `run(Launch, Transport, Channels)` is the
whole conversation, a future that reads the harness's stdout and writes its stdin through the boxed `Transport` and returns when
the session is over. `Channels` carries the `flume::Receiver<Command>` (prompt, interrupt,
shutdown, in call order) and the `flume::Sender<Event>`. `driver_for` is the only place that
maps a `HarnessKind` to a driver. A `Plan` may carry a `refusal` (a `Failure`) instead of
arguments: `start_in` then launches nothing and returns a session that has already ended, whose
stream holds `TurnFinished(Failed(..))` and `Exited(Some(message))`, so a failure the app handles
by `FailureKind` (a resume that cannot work: `SessionLost`) never needs a process. Tests run a
driver on `tokio::io::duplex` halves without a process; `start_in` takes a registry and an `Environment` so tests start real
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
- Waiting for the stderr reader (`process::drain`) leaves the reader in place until it has
  finished or been given up on: the supervisor races the driver against `Process::wait`, and the
  loser's future is dropped mid-drain, which must not cost the next drain the reader (the
  `Exited` text would lose the stderr tail).
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
- Notifications and answers reach a `jsonrpc::Connection` user through different channels: a
  notification read before an answer is already queued when the answer resolves, so a user that
  needs their order drains `Connection::try_next` after the answer.
- `jsonrpc::Options::tag_version = false` leaves the `"jsonrpc":"2.0"` field out for peers
  (Codex) that omit it. Incoming request ids may be numbers or strings; ours are numbers.
  `Connection::close` flushes what was queued and ends the writer; dropping it aborts both tasks.
- `probe` runs `<command> --version` with a 10 s limit in the temp folder and returns the first
  non-empty line (stdout, else stderr).

## Brief and prompt texts

- The patch is never in the brief; the agent reads it through git. A brief names the kind of
  review (merge or pull request, branch comparison, uncommitted changes). Uncommitted changes have
  no head commit id, so the text sends the agent to `git status` / `git diff HEAD`, and
  `disk_matches_head` is ignored for them (the working tree is the subject).
- Repository and forge text (description, commit list, comments) sits between
  `<<< untrusted: <label> >>>` and `<<< end untrusted >>>` lines. `<<<` and `>>>` inside any
  rendered content become `‹‹‹` and `›››`, so content cannot produce a boundary; the markers are
  fixed, not random, because the output must be reproducible for snapshot tests.
  Overflow notices ("and N more ...", threads not shown) are written after the closing marker.
- Single-line fields (title, author, branches, paths, subjects) lose control characters and
  line breaks and are cut at 1000 characters; multi-line fields keep `\n` and tab. All limits
  count `char`s and cut on a char boundary, and a cut says how many characters were left out.
- Limits: description 20 000 characters, 200 commits, 500 entries per file list, a comment body
  2 000, all threads together 40 000. Threads go in the given order, later ones are dropped whole
  with a count; a first thread that alone exceeds the budget is cut instead of dropped.
- A thread's digest is its id, resolved flag, position and every note's author and body in full
  (not the truncated rendering), as compact JSON; the fingerprint is FNV-1a 64 of it, stable
  across runs and Rust versions so the app may persist it. `changed_since` compares by thread id.
- `instructions` is the Appendix text with the answer language (a display name, empty means
  English) and the disk rule; the head and base ids come from the brief, so after a head change
  the app sends a fresh brief along with `update_task`.
- A selection is sent as the user's text, `Selected: [<path> <side><start>-<end>](diff:...)` and
  the text in a code fence one backtick longer than any run inside it (at least three); the link
  comes from `review_core::diff_link::format`.

## Decisions

- `process-wrap` (tokio) for the process group and the job object instead of `nix` and
  `taskkill`: no `unsafe` in the crate and one code path for both platforms; the soft shutdown
  signals through its `signal`.
- Modules are `pub` so drivers and tests reach them; the app is expected to use only the
  contract in `lib.rs`.
- `Error::Unsupported` is a unit variant that no driver produces any more; it stays part of the
  contract the app matches on. No variant was added for a lost session, the app's contract for it
  is `FailureKind::SessionLost` on the event stream.
- No `futures` crate: the shutdown futures join `JoinHandle`s and a `watch` channel.
- Drivers sit on `AsyncRead` + `AsyncWrite`, not on `Child`, so recorded sessions replay over
  `duplex` the same on every OS.
- `agent` depends on `review_core` only for `Side` and the `diff:` encoder, so the link a prompt
  asks for and the one the app parses cannot drift apart.
