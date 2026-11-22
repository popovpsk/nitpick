# agent::pi

The Pi driver: builds the command line and instructions file for `pi --mode rpc`, checks the
session with `get_state`, runs the turn loop over Pi's line-delimited JSON pipes and turns its
events into `Event`s. GPUI-free.

The driver is written from the Pi 1.6.0 documentation (docs.pi.dev: cli, rpc, rpc-commands, json,
message-types, security, sessions) and has not been run against a real CLI. The tests replay
synthetic transcripts written in the test code, so every wire fact below is documented, not
verified.

Not here: starting or killing the process (`session`, `process`), the instruction text
(`Launch.instructions` comes from the app).

## Files

- `mod.rs` — `PiDriver` (`Driver::plan` and `run`), `Limits`, `arguments`, `valid_session_id`, the
  handshake, the turn loop, the bounded waits and the end-of-session handling.
- `wire.rs` — lenient frame types (every field optional, a mistyped field reads as absent) and the
  lines written to stdin.
- `normalize.rs` — `Normalizer`: frames to `Event`s, the turn state, tool titles, the outcome at
  `agent_settled`, the answer to extension dialogs.
- `classify.rs` — failure kind from error text.

## Invariants and spec

- Command line after the user's command: `--mode rpc --no-themes --no-approve --no-skills
  --no-prompt-templates --tools read,grep,find,ls,bash --session-id <id> --append-system-prompt
  <scratch>/instructions.md [--model <m>]`. The instructions are written to `scratch` and only the
  path is passed: Windows `.cmd` wrappers break long or quoted arguments. The flags repeat on every
  launch, resume included; the system prompt is passed each time because Pi's handling of it on a
  resumed session is not documented. Whether it applies there is unverified, so the app does not
  rely on it: it sends changed instructions as a message block on a resumed session.
- `--no-approve` is always passed: Pi would otherwise load trust-gated project resources
  (`.pi/` extensions, settings, prompts) for a user whose `defaultProjectTrust` is `always` or who
  saved trust for the repository. Never `-a`. Extensions are not disabled (`--no-extensions` would
  also drop the user's own provider extensions), so the user's global extensions and settings,
  `AGENTS.md` and `CLAUDE.md` files still load; the instructions already tell the agent to treat
  repository text as untrusted data.
- There is no sandbox and no confirmation in Pi: `bash` can write files and use the network with
  the rights of the user. That is the owner's accepted decision. The tool list omits Pi's `edit`
  and `write` tools and nothing else stops a write; the instructions forbid it. On native Windows
  the `bash` tool needs Git Bash.
- Session ids: a new session is started with `--session-id nitpick-<uuid>`; a resume passes the
  stored id as `--session-id`. Pi accepts letters, digits, `.`, `_`, `-`, starting and ending with
  a letter or digit; `plan` returns `Plan::refusal` (`SessionLost`, no process) for a stored id that
  breaks the rule, which also keeps an id from being read as a flag.
- `--session-id` creates the session silently when the id does not exist, so the driver sends
  `get_state` first (before any prompt; prompts sent meanwhile are queued). When a resume was
  requested and `messageCount` is 0, absent or the answer has no `data`, the session is lost:
  nothing is written to Pi, every queued prompt gets `TurnFinished(Failed(SessionLost))` (exactly
  one such event when none is queued), no `Event::Session` is emitted, and the driver returns. The
  app then clears the stored id and sends its request with the brief to a new session. A session
  Pi creates but never completes a turn in has no file (Pi writes it with the first assistant
  message), so it counts as lost on the next resume.
- `Event::Session { id }` is emitted once, after `get_state` passes: Pi's `sessionId`, else the
  requested id. A new session whose state carries no `sessionId` emits none (it works, but cannot
  be resumed). A failed `get_state` (`success: false`) fails the queued prompts with the error
  text, classified.
- Commands (one JSON object per line, each with an `id`): `prompt {message}` per turn,
  `get_state`, `clear_queue`, `abort`. A prompt is sent only when no turn runs, so Pi's own queue
  and `streamingBehavior` are never used. The `prompt` response only acknowledges acceptance:
  `success: false` fails the turn at once with the error text (classified); `data.disposition:
  "handled"` (an extension consumed the text, no run starts) completes it at once; `started` or
  `queued` keep it open.
- Events: `message_update.assistantMessageEvent` `text_delta` / `thinking_delta` (`delta`, empty
  ones dropped) are `Text` / `Thinking`; other nested types are ignored. At an assistant
  `message_end` the whole content text and thinking are used only for a kind no delta streamed in
  that message (the flag resets at `message_start`); an errored or aborted message contributes no
  text. `tool_execution_start` / `tool_execution_end` are `ToolStarted` / `ToolFinished` (`isError`
  is `failed`); a duplicate start, an end without a start and `tool_execution_update` are
  dropped. The title is `args.command` for `bash`, `args.pattern` (else `path`) for `grep` and
  `find`, `args.path` (else `pattern`) for the rest, else the tool name, on one line and cut at 120
  characters. `auto_retry_start` is a `Notice`. Events that arrive while no turn runs are ignored.
- The end of a turn is `agent_settled`, never `agent_end`: retries, compaction and queued input go
  on after `agent_end`. Only the state at `agent_settled` decides the outcome: `aborted: true` in
  it or an aborted last assistant message is `Interrupted`; otherwise the last assistant
  `message_end` with `stopReason` `error` (its `errorMessage`; a failed `auto_retry_end`'s
  `finalError` when no message carried one) is `Failed`; any later assistant message with another
  stop reason clears the error, so an error followed by a successful retry completes. `length`
  counts as completed. Open tools are closed as failed when a turn ends.
- Failure classification: Pi documents no error code, so the kind comes from the text only
  (`NotLoggedIn`: "no api key", "invalid api key", "unauthorized", "authentication", "oauth",
  "/login", 401; `UsageLimit`: "rate limit", "quota", "usage limit", "too many requests", 429;
  `ContextTooLong`: "context length", "context window", "prompt is too long"; `ModelNotFound`:
  "model not found", "no models match", "unknown model"; a bare number must not touch letters or
  digits), else `Other`. Classification never yields `SessionLost`.
- Extension dialogs: every `extension_ui_request` with an id is answered at once, in any phase,
  with `{"type":"extension_ui_response","id":<id>,"cancelled":true}`, because an unanswered dialog
  wedges the turn. The fire-and-forget methods `notify`, `setStatus`, `setWidget`, `setTitle`,
  `set_editor_text` expect no reply and get none; any other method name, known or not, is
  answered.
- Stop: `Interrupt` while a turn runs writes `clear_queue`, then `abort` (`abort` alone would let
  Pi go on with its queue), once per turn, from the moment the `prompt` command is written (also
  before Pi's `response` to it). The turn then ends at `agent_settled` (`aborted: true`) as
  `Interrupted`, and queued prompts survive and go out after it. With no turn active, `Interrupt`
  cancels the oldest queued prompt instead: it is never written to Pi and gets one
  `TurnFinished(Cancelled)` at once, later prompts stay queued, and with nothing queued it is a
  no-op. If the turn has not ended `Limits::stop` (15 s) after the request, the driver emits a
  `Notice`, ends the running turn as `Interrupted`, fails every queued prompt (`Failed(Other)`)
  and returns; the supervisor kills the process and the next session resumes by id.
- Bounded waits (`Limits`, injectable): `state` (30 s) for the `get_state` answer, `stop` (15 s)
  above. A silent Pi fails the queued prompts with "Pi did not answer get_state within <limit>".
  A turn that streams nothing for a long time is not cut off.
- Prompts: one turn at a time. A `Prompt` during a handshake or a turn is queued and sent after
  the turn's `agent_settled`. The command channel is polled in every phase, including while the
  `get_state` answer is awaited, and drained before a queued prompt is written, so an `Interrupt`
  never falls into the gap between the state check and the write.
- End of stdout (or a failed write) fails the running turn (open tools closed as failed) and every
  queued prompt, one `TurnFinished(Failed(Other))` each; nothing is emitted between turns. A
  handshake that fails or ends this way emits one lone failure whenever no prompt is running or
  queued, even after an earlier cancel: it may name a prompt that is on its way but not yet in the
  command channel, and a spare failure is harmless where a missing one is not. The command channel
  is drained first: the loop picks between a ready command and a ready end of stream at random, so
  without the drain a prompt submitted just before the end would never get its `TurnFinished`. An
  `Interrupt` found in that drain with no turn active cancels the oldest queued prompt
  (`TurnFinished(Cancelled)`) before the rest are failed; with a turn active it is ignored there,
  since that turn ends with its own outcome.
  `Shutdown` (or a dropped command channel) returns with no further events. The driver never emits
  `Exited`.
- Parsing is tolerant: unknown event types and fields are ignored; a line that is not JSON is
  logged at debug (first 200 characters) and skipped; invalid UTF-8 is replaced by the framing
  layer.

## Open points

- `--tools` is documented both as the final allowlist and as leaving MCP tools in place unless
  an entry starts with `mcp__`; if the second holds, the user's own MCP servers stay available to
  the agent (project ones are gated by `--no-approve`).
- Whether `--append-system-prompt <path>` is read as a file in RPC mode on every launch, and
  whether `messageCount` counts the whole session or only the active branch, is undocumented.
- Whether `get_state` is answered before extensions finish loading is unknown, so `Limits::state`
  is generous.
- Whether `sessionId` equals the id passed to `--session-id` is assumed; Pi's reported id is the
  one stored.
- No error text is documented beyond old changelog lines ("No API key found", an OAuth failure
  message); the classifier's hints are guesses at provider messages Pi passes through.

## Model list

- `catalog::pi_models` (unverified, written from public documentation): after the usual launch
  flags, `{"id":"models-1","type":"get_available_models"}`; the answer is the `response` frame
  with that `command`, `data.models[]` with `id`, `name`, `provider`. The stored value is
  `provider/id` (what `--model` accepts as a pattern, also unverified) and the label `name`.

## Decisions

- Native `pi --mode rpc` rather than `--mode json` per turn: one process serves every turn and
  the stop is a command, not a signal.
- A missing session fails the turn with `SessionLost` instead of continuing in the fresh session
  Pi created: the app owns the decision to re-brief, as with Claude Code and ACP.
- A missing `messageCount` counts as an empty session: a needless re-brief is cheaper than an
  agent that silently lacks the conversation the panel still shows.
- The driver queues prompts itself instead of using Pi's `steer` / `followUp` queue, so all
  harnesses share one queueing contract.
- Pi's `edit` and `write` tools are left out, but `bash` stays: the review needs `git`. The owner
  accepted that this is no protection against a write.
