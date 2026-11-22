# agent::codex

The Codex driver: runs `<command> app-server` as a read-only JSON-RPC peer over stdio, opens or
resumes one thread per session, sends one `turn/start` per prompt and turns the server's
notifications into `Event`s. GPUI-free.

The driver is written from the documentation of Codex 0.171 and from wire captures of CLI
0.146.1 made outside this repository; it has not been run against a real CLI. The replay tests
use synthetic transcripts, so every wire fact below is documented, not verified.

Not here: starting or killing the process (`session`, `process`), the JSON-RPC framing
(`jsonrpc`), the instruction text (`Launch.instructions` comes from the app).

## Files

- `mod.rs` — `CodexDriver` (`Driver::plan` and `run`), `Limits`, `arguments`, the handshake, the
  turn loop, the bounded waits and the end-of-session handling.
- `wire.rs` — request parameter builders, the answers to server requests, lenient types for the
  notifications (every field optional, a mistyped field reads as absent).
- `normalize.rs` — `Normalizer`: notifications to `Event`s, tool titles, turn outcome.
- `classify.rs` — error classification, resume-error classification, the version rule.

## Invariants and spec

- Command line after the user's command: `app-server`, nothing else. The session needs no scratch
  folder and its `Plan` has no environment.
- Wire: JSON-RPC without the `"jsonrpc"` tag (`Options::tag_version = false`). Order:
  `initialize` (`clientInfo` `nitpick`, `capabilities: {"experimentalApi": true}`), the
  `initialized` notification with `params: {}`, `thread/start` or `thread/resume`, then one
  `turn/start` per prompt; `turn/interrupt {threadId, turnId}` stops a turn.
- Read-only is requested twice: `approvalPolicy: "never"` and `sandbox: "read-only"` in
  `thread/start` and `thread/resume`, `approvalPolicy: "never"` and
  `sandboxPolicy: {"type":"readOnly"}` in every `turn/start`. Never `untrusted` (retired by
  Codex), never `thread/shellCommand` (runs outside the sandbox). `summary: "auto"` is in every
  `turn/start`: without it no reasoning summary streams.
- `thread/resume` carries `threadId`, `excludeTurns: true` and the same settings as
  `thread/start`; the model and the developer instructions are sent again on resume (whether Codex
  applies them to an existing thread is unverified; the app also sends changed instructions as a
  message block on a resumed session).
- The app's instructions go in `developerInstructions` (omitted when empty); `baseInstructions`
  is never set because it replaces Codex's own system prompt. No `config` overrides are sent:
  the repository's `AGENTS.md` and a user's configuration keep their effect (see Open points).
- `cwd` is the launch folder made absolute. The model is sent only when set.
- `Event::Session` is emitted once, when `thread/start` or `thread/resume` has answered, with
  `thread.id` (the requested id when a resume answer carries none).
- Every server request is answered at once, in any phase: `item/commandExecution/requestApproval`
  and `item/fileChange/requestApproval` get `{"decision":"decline"}`,
  `item/permissions/requestApproval` gets `{"permissions":{},"scope":"turn"}`,
  `item/tool/requestUserInput` gets `{"answers":{}}`, `mcpServer/elicitation/request` gets
  `{"action":"decline"}`, anything else gets error `-32601`. An unanswered request wedges the
  turn.
- Notifications about another thread (sub-agents) or another turn id are ignored, as is
  everything while no turn runs: a stale or duplicate `turn/completed` never settles a turn.
- Text: `item/agentMessage/delta` (`delta` or `textDelta`) is `Text`. A completed `agentMessage`
  item whose id streamed no delta contributes its whole `text`. Consecutive messages arrive
  without a separator, so `\n\n` is added in front of the first text of the next message; the
  separator is never trailing, so stored text has no dangling blank lines, and it does not carry
  over to the next turn.
- Thinking: `item/reasoning/summaryTextDelta` is `Thinking`; a change of reasoning item or
  `summaryIndex` after some thinking adds a leading `\n\n`. Raw reasoning deltas are ignored.
- Tools: `item/started` and `item/completed` of `commandExecution` (or `command_execution`) are
  `ToolStarted` and `ToolFinished`. The title is the distinct `commandActions[].command` values
  joined with ` && `, because `command` carries the shell wrapper (`zsh -lc '…'`); without actions
  the wrapper is stripped from `command`, else the title is `command`. A tool fails when its
  status is `failed` or `declined` or its exit code is not 0 (`exitCode` or `exit_code`). A
  completion without a start emits both; a duplicate is dropped. Tools still open at the end of
  a turn are closed as failed.
- `turn/completed`: `turn.status` `interrupted` is `Interrupted`, `failed` is `Failed`, `completed`
  is `Completed`; with no usable status, an error makes it `Failed`. The legacy `turn/failed` and
  `turn/aborted` are not read.
- `error` notifications (nested `error` object, or the older flat fields): `willRetry: true` is a
  `Notice`; otherwise the error is held and used as the failure of the turn's `failed` completion
  when the completion carries none.
- Failure classification uses the code first (`codexErrorInfo`: a string, or an object whose key
  is the variant and whose value may hold `httpStatusCode`; case and `_` ignored), then text as a
  hint: `NotLoggedIn` (`unauthorized`, 401, "not logged in", "codex login", "invalid api key"),
  `UsageLimit` (`usageLimitExceeded`, `rateLimitExceeded`, 429, "usage limit"),
  `ContextTooLong` (`contextWindowExceeded`, `context_length_exceeded`), `ModelNotFound`
  (`model_not_found`, or "model" with "does not exist"), else `Other`. Classification never
  yields `SessionLost`.
- Resume errors: "no rollout found", "thread not found", "session not found", "conversation not
  found", "invalid session id", "invalid thread id", and "failed to read thread" / "rollout at"
  with an empty, non-UTF-8 or unparseable history are `SessionLost`; "is archived" runs
  `thread/unarchive {threadId}` and resumes once more (a second "archived" is an ordinary
  failure). A bare "not found", a permission problem, a lock, a busy disk, a configuration error
  are ordinary failures: a new thread is never started silently.
- Version rule: the minimum is 0.146.0, the oldest CLI whose wire this driver maps (nested
  `error` notifications). The version is read from `initialize`'s `userAgent`
  (`<client name>/<version> (<platform>) …`), else from `thread.cliVersion` in the thread
  answer, so no second process runs; a pre-release counts as its release. An unknown version
  (a wrapper that reports nothing) gets the benefit of the doubt. A refused CLI fails before
  any thread exists and emits no `Event::Session`.
- Handshake failure: the pending and queued prompts each get one `TurnFinished(Failed(..))`; with
  none pending exactly one is emitted whatever happened before (even after a cancelled prompt),
  because a prompt may be on its way that the driver has not read yet. The rule can emit one
  failure for which no prompt is owed, never drop one that is owed: the app turns a failure with no turn
  into a failed panel state, but a turn ended only by `Exited` loses its classification and the
  automatic recovery from `SessionLost`. The driver then returns and the supervisor adds `Exited`.
- Bounded waits (`Limits`): `initialize` 30 s, thread calls 60 s, `turn/start` 30 s,
  `turn/interrupt` 10 s, and 15 s from an interrupt request to the end of the turn. A handshake
  or `turn/start` timeout fails the turn and ends the session. An interrupt that is not honored
  emits a `Notice`, ends the running turn as `Interrupted` and ends the session; the next
  session resumes the thread. A turn that streams nothing for a long time is not cut off.
- Prompts: one turn at a time. A `Prompt` during a turn, a handshake or a pending call is queued
  and sent after the turn ends. The command channel is polled in every phase, including while a
  handshake request or `turn/start` is awaited. `Interrupt` with a turn active (from the moment
  `turn/start` is written) is remembered and sent as `turn/interrupt` once the turn id is known
  (from the `turn/start` answer or `turn/started`); queued prompts survive it. `Interrupt` with no
  turn active cancels the oldest queued prompt: it is never sent to the CLI and gets one
  `TurnFinished(Cancelled)` (a turn whose `turn/start` was written ends `Interrupted`); later
  prompts stay queued; with nothing queued it is a no-op. An `Interrupt` still unread when the
  session ends is applied the same way before the remaining prompts get their failure. A
  refused `turn/interrupt` is logged and not fatal.
- A rejected `turn/start` fails that turn with the classified error and the session goes on.
- End of stdout, a failed write or a closed connection after the handshake fails the running turn
  (open tools closed as failed) and every queued prompt, one `TurnFinished(Failed(Other))` each;
  nothing is emitted between turns. The command channel is drained first: the loop picks between
  a ready command and a ready end of stream at random, so without the drain a prompt submitted
  just before the end would never get its `TurnFinished`. `Shutdown` (or a dropped command
  channel) returns with no further events. The driver never emits `Exited`.
- Parsing is tolerant: unknown notifications and fields are ignored; a line that is not JSON is
  skipped by the framing layer.

## Open points

- `thread/start` accepts a `config` object (project documents, MCP servers, features), but
  whether each key is valid on 0.171 is undocumented, so none is sent. Whether the repository's
  `AGENTS.md` is read under `read-only` and whether `project_doc_max_bytes: 0` would drop the
  user's own is unknown.
- The two Codex documentation pages disagree on whether commands (`git log`, `cat`) run under
  `read-only` with `approvalPolicy: "never"`; the driver relies on them running.
- `developerInstructions` is assumed to be appended to Codex's own prompt, not to replace it.
- No signal for an unknown model is documented; `ModelNotFound` is a text hint on the OpenAI API
  error.

## Model list

- `catalog::codex_models` (unverified, written from public documentation): after `initialize` /
  `initialized`, `model/list` with `{}` and then `{cursor}` until `nextCursor` is null (at most
  20 pages); `hidden` entries are skipped, the value passed back is `model` (else `id`), the
  label `displayName`, `isDefault` marks the default.

## Decisions

- Native `codex app-server` rather than `codex exec --json`: only the server streams text deltas.
- Waits are bounded for every request but the turn itself, so a silent server costs the user an
  interrupt, not the session.
- `mcpServer/elicitation/request` is declined rather than rejected with `-32601`, as the Codex
  documentation lists it among the requests that need an answer.
- Titles are not stripped of the shell wrapper by parsing: the actions list is preferred and the
  wrapper strip is only a fallback.
