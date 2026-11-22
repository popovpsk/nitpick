# agent

Runs the review assistant: starts an installed agent CLI (Claude Code, Codex, an ACP agent,
Pi) as a child process in the repository, feeds it prompts and turns its output into one
stream of `Event`s. It holds the shared machinery (process handling, line reading, JSON-RPC
framing, the session and registry), the seam a harness driver plugs into, the drivers
(Claude Code, Codex, the ACP agents OpenCode, DeepSeek and any other, Pi) and the texts the app
sends (`prompt`, `brief`). GPUI-free.

Not here: UI, settings and persistence (`nitpick`, `app_db`), filling a `Brief` from review data
(`nitpick`), the `diff:` link format (`review_core`, which `prompt` calls).

## Files

- `lib.rs` — the contract: `HarnessKind`, `AcpFlavor`, `Launch`, `Event`, `Outcome`,
  `Failure`, `FailureKind`, `Error`, `probe`; re-exports `Session`, `shutdown_all` and `quit`. The app
  uses only these.
- `process.rs` — command splitting, login-shell environment snapshot, environment merge and
  scrub, program lookup, `Invocation`, `Process` (spawn in a process group / job object, soft
  shutdown, stderr tail), the quit-time reaper (`reap`, `reaper_args`), `drain`, `probe_version`,
  `on_runtime`.
- `lines.rs` — `Lines`: newline framing over any `AsyncRead` that survives bad UTF-8 and
  skips lines over 32 MiB.
- `jsonrpc.rs` — line-framed JSON-RPC 2.0 over an `AsyncRead` + `AsyncWrite`: `connect`,
  `Client` (requests, notifications, replies to server requests), `Connection` (incoming
  notifications and server requests).
- `registry.rs` — `Registry` of live sessions, `shutdown_all` and `quit`.
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
- Quitting the app (`agent::quit()`, called synchronously from the quit handler) cannot use the
  soft shutdown: gpui waits 200 ms for quit handlers and `net_runtime` is never dropped, so
  nothing in-process survives to step through 3 s and 2 s. On Unix `quit` spawns one detached
  `/bin/sh` (own process group, stdio to `/dev/null`, `HUP` ignored, `/` as cwd) that runs the same
  ladder after the app is gone: it sleeps 3 s, sends SIGTERM to every live session's group,
  sleeps 2 s, sends SIGKILL. `quit` first asks every driver to shut down (which drops stdin), then
  starts the reaper; the pipes close with the app at the latest, so the harness sees EOF at once
  and a polite one (Claude Code finishing a token refresh) exits well inside the 3 s. `quit` does
  no file-system access of its own (a hung mount on the user's `PATH` must not block quitting):
  it only builds the reaper's `PATH` and spawns `/bin/sh`. That `PATH` is the app's merged one
  (`Environment::cached()`, which never waits for the login-shell snapshot) with `/usr/bin` and
  `/bin` appended when missing, so `sleep` is found wherever the user has it (NixOS has none in
  `/usr/bin`) and when the snapshot and `PATH` are empty; `spawn_reaper` takes an exact `PATH`.
  The wait fails closed inside the detached script: it exits before any signal if `sleep` is not
  found (`command -v`), and after either `sleep` that fails. No portable wait exists in `sh`
  without `sleep` (`read -t` is not POSIX), so without it a harness only gets stdin EOF: a leak
  is possible, an early kill that costs the user's login is not.
  The reaper's script is a constant; the only data it receives are the two delays and the group
  ids, as arguments that are plain decimal numbers (ids outside 2..=`i32::MAX` are dropped), never
  interpolated into the script. A group id is its leader's pid and stays reserved while any member
  lives; once the group is empty the id may be reused, and within the 5 s after quit the reused
  id would also have to become a group leader, which is accepted as unrealistic (the kill of an
  empty group is a no-op). A process that left the group (its own `setsid`) is out of reach, as
  in the in-process ladder. On Windows the job object is created with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (process-wrap sets it because `KillOnDrop` is wrapped), so
  the OS terminates the whole tree when the app's handle closes; that is immediate, without the
  3 s wait, and `quit` there only asks the drivers to shut down.
- A turn ends with one of `Outcome::Completed`, `Interrupted` (a turn that had been sent was
  stopped), `Cancelled` (the prompt was dropped before anything was written to the harness, so
  the harness never saw it) or `Failed`. The app decides a brief was delivered from the first
  two only.
- Dropping the last `Session` clone requests the same shutdown; the registry keeps only the
  control handle, so it never keeps a session alive. `shutdown()` aborts the supervisor if the
  session has not ended in 10 s (the drop of `Process` then kills the group).
- `start_in` removes `Launch.scratch` when it returns an error or a refusal, but only if the
  call created the folder: a pre-existing one (and its files) is left alone, and parents created
  on the way stay. A started session keeps its folder; removing it later is the app's job.
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
- Every value that comes from the repository or the forge sits between
  `<<< untrusted: <label> >>>` and `<<< end untrusted >>>` lines: the change header (reference,
  title, URL, author, state, branch names), description, commit list, file lists (changed,
  hidden, viewed) and comments with their positions. Every other line of a brief is written by
  Nitpick (headings, counts, the kind, the "files on disk" statement, the git commands, overflow
  notices); the preamble says so, and a test checks it. `<<<` and `>>>` inside any rendered
  content become `‹‹‹` and `›››`, so content cannot produce a boundary; the markers are fixed,
  not random, because the output must be reproducible for snapshot tests. Overflow notices
  ("and N more ...", threads not shown) are written after the closing marker.
- Commit ids reach the text outside a fence only through `commit_id`: ASCII hex, at most 64
  characters, else the id counts as unknown, the git command is not written, and the text points
  at the branch names in the change block. `update_task` follows the same rule.
- Single-line fields (title, author, branches, paths, subjects) lose control characters and
  line breaks and are cut at 1000 characters; multi-line fields keep `\n` and tab. All limits
  count `char`s and cut on a char boundary, and a cut says how many characters were left out.
- Limits: description 20 000 characters, 200 commits, 500 entries per file list, a comment body
  2 000, all threads together 40 000. Threads go in the given order, later ones are dropped whole
  with a count; a first thread that alone exceeds the budget is cut instead of dropped.
- A thread's digest is its id, resolved flag, position and every note's author and body in full
  (not the truncated rendering), as compact JSON; the fingerprint is FNV-1a 64 of it, stable
  across runs and Rust versions so the app may persist it. `changed_since` compares by thread id.
- `instructions` is the fixed text (read-only rules, the untrusted-data rule, the `diff:` link
  format) with the disk rule and the answer language (a display name, empty means English); the
  head and base ids come from the brief, so after a head change the app sends a fresh brief
  along with `update_task`.
- A selection is sent as the user's text, `Selected: [<path> <side><start>-<end>](diff:...)` and
  the text in a code fence one backtick longer than any run inside it (at least three). The path
  is repository text, so in what the agent receives it appears only percent-encoded (label and
  link target alike, from `review_core::diff_link::format`); `selection_link`, the marker shown
  to the user in the input, keeps the readable escaped path as the label.

## Decisions

- `process-wrap` (tokio) for the process group and the job object instead of `nix` and
  `taskkill`: no `unsafe` in the crate and one code path for both platforms; the soft shutdown
  signals through its `signal`.
- Modules are `pub` so drivers and tests reach them; the app is expected to use only the
  contract in `lib.rs`.
- `Error::Unsupported` stays in the contract the app matches on, though every `HarnessKind` has a
  driver. A lost session is not an `Error`: the app's contract for it is
  `FailureKind::SessionLost` on the event stream.
- No `futures` crate: the shutdown futures join `JoinHandle`s and a `watch` channel.
- Drivers sit on `AsyncRead` + `AsyncWrite`, not on `Child`, so recorded sessions replay over
  `duplex` the same on every OS.
- `agent` depends on `review_core` only for `Side` and the `diff:` encoder, so the link a prompt
  asks for and the one the app parses cannot drift apart.
