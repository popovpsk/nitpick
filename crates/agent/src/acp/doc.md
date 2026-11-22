# agent::acp

The generic Agent Client Protocol driver: runs the user's command (`opencode acp`, `dsh acp`, a
custom agent) as a JSON-RPC peer over stdio, opens or resumes one session, sends one
`session/prompt` per prompt and turns `session/update` notifications into `Event`s. One driver
serves three flavors (OpenCode, DeepSeek, Generic) that differ only in `flavor.rs`. GPUI-free.

Not here: starting or killing the process (`session`, `process`), the JSON-RPC framing
(`jsonrpc`), the instruction text (`Launch.instructions` comes from the app), the `acp`
argument itself (it is part of the user's command; the plan adds no arguments).

## Files

- `mod.rs` — `AcpDriver` (`Driver::plan` and `run`), `Limits`, `VersionProbe`,
  `probe_program_version`, the handshake, the turn loop, the bounded waits and the end-of-session
  handling.
- `wire.rs` — request parameter builders, `Capabilities`, `config_option`, lenient types for
  `session/update` (every field optional, a mistyped field reads as absent).
- `normalize.rs` — `Normalizer`: updates to `Event`s, tool lifecycle, paragraph separators, the
  cancellation-note filter, the turn outcome.
- `permission.rs` — pure: tool kind to verdict, verdict and options to the reply.
- `flavor.rs` — per-flavor environment, config dialect, resume method, error classification.

## Invariants and spec

- Protocol v1. `initialize` carries `clientCapabilities` `{"fs":{"readTextFile":false,
  "writeTextFile":false},"terminal":false}` and `clientInfo` `nitpick`; the agent's own tools
  do the work. Then `session/new {cwd, mcpServers: []}` (`cwd` absolute, `mcpServers` required),
  or a resume; `session/prompt {sessionId, prompt: [text blocks]}` per turn; `session/cancel
  {sessionId}` (a notification) to stop one. JSON-RPC carries the `"jsonrpc":"2.0"` tag.
- `Event::Session` is emitted once, after the session exists and the model and mode options are
  set, with the id from the answer (the requested id when a resume answer carries none).
- Resume (`Launch.resume` set): `sessionCapabilities.resume` declared selects `session/resume`;
  else `loadSession` selects `session/load` (DeepSeek never loads); neither is a lost session.
  Both take `{sessionId, cwd, mcpServers}` and the `cwd` must equal the original one.
  `session/load` replays the history as updates before its answer; no turn is running then, so the
  replay never becomes an event. Waits: 60 s for new and resume, 90 s for load.
- A lost session is reported by failing the turn with `FailureKind::SessionLost`, never by
  silently starting a fresh session: the app clears the stored id and sends the request again to a
  new session. It is raised only while resuming: for no usable method, `-32002`, a message
  containing "session not found" (any flavor), and for OpenCode the answer `-32603` whose
  `data.service` is `"session"`. A timeout or a broken transport while resuming is an ordinary
  failure, never a lost session. No `Event::Session` precedes it.
- Instructions: if the agent declares `sessionCapabilities.systemPrompt`, `session/new` and the
  resume call carry `systemPrompt {mode: "append", text}` (no agent in the recordings declares it,
  so the `text` field name is not verified). Otherwise the instructions are the first text block
  of the first prompt of a new session, and the block is dropped only after a prompt got an
  answer, so a failed first prompt is retried with it. A resumed session never gets them again.
  Empty instructions are not sent.
- Options: an option with `category: "model"` and a `Launch.model` that is not already its
  `currentValue` is set through `session/set_config_option {sessionId, configId, value}`; values
  are opaque strings (DeepSeek's are JSON text). A refused model fails the session as
  `ModelNotFound`. The options come from the answer of `new` / `resume` / `load`.
- Permissions: a `session/request_permission` is answered at once, in any phase. A tool kind of
  `edit`, `delete` or `move` is rejected, everything else (read, search, execute, think, fetch,
  switch_mode, other, an unknown or missing kind) is allowed. The kind is the request's own, else
  the one remembered from the earlier `tool_call` with the same `toolCallId`; a write kind from
  either source rejects. The option is picked by its `kind` field (`allow_once` for allow,
  `reject_once` for reject; hyphens read as underscores), never by id or position, and never
  `allow_always` or `reject_always`; with no such option the answer is `cancelled`.
- Any other agent request (`fs/*`, `terminal/*`, `elicitation/create`, `_`-prefixed, unknown)
  gets error `-32601`; an unanswered request wedges the agent. Notifications other than
  `session/update`, updates with another `sessionId`, `_`-prefixed and unknown update kinds and
  methods are ignored.
- Text and thinking: `agent_message_chunk` and `agent_thought_chunk` text deltas are `Text` and
  `Thinking`. A new `messageId` after text was emitted puts `\n\n` in front of the first delta (one
  channel each); without ids, text after a tool call gets it, so stored text has no joined
  paragraphs and no dangling blank lines. `user_message_chunk` and everything else are ignored.
- Tools: a call is `pending` while its arguments stream and its title is only the tool's name.
  `ToolStarted` is emitted once per call, on the first `in_progress`, `completed` or `failed`
  status, with the latest title seen (the command line for a shell call); repeated updates add
  nothing. `ToolFinished` is emitted once, on `completed` (`failed: false`) or `failed`. A call
  that goes straight to `completed` emits both; an update for an id without a `tool_call` is the
  start of that call. Later updates of a closed call are dropped. Titles are one line, cut at 120
  characters; an empty title becomes the tool kind, else `tool`. Started calls still open when the
  turn ends are closed as failed.
- Turn end: the `session/prompt` answer. `stopReason` `cancelled` is `Interrupted`, `refusal` is
  `Failed(Other)`, anything else `Completed`; an RPC error fails the turn (classified, the session
  goes on). Notifications written before the answer are read before the turn ends: the reader
  hands notifications and answers over different channels, so the driver drains the notification
  queue when the answer arrives.
- Prompts: one turn at a time. A `Prompt` during a turn or a pending call is queued and sent
  after the turn ends. `Interrupt` counts only while a turn runs: it sends one `session/cancel`
  and starts the cancel wait; queued prompts survive it. The agent's text after a cancel that is
  the note "The request was cancelled by the client." (either spelling, split over chunks) is not
  `Text`; any other text after a cancel is released whole.
- Watchdog: no message of any kind from the agent for 120 s while no tool call is open (a call is
  open from its first update to its `completed` / `failed`, pending included) fails the turn with
  "The assistant is not responding." and ends the session; queued prompts get the same failure.
  After `session/cancel` the agent has 12 s to answer the prompt; otherwise a `Notice` is emitted,
  the turn ends as `Interrupted`, queued prompts fail and the session ends, so the next session
  resumes in a new process. `Limits` holds every duration (defaults: initialize 30 s, session
  60 s, load 90 s, option 15 s, idle 120 s, cancel 12 s).
- End of stdout, a failed write or a closed connection after the handshake fails the running turn
  (open tools closed as failed) and every queued prompt, one `TurnFinished(Failed(Other))` each;
  nothing is emitted between turns. A failure before the session is ready fails each queued
  prompt, and exactly one turn when none is queued, so the app always learns it. The command
  channel is drained first. `Shutdown` (or a dropped command channel) returns with no further
  events. The driver never emits `Exited`.
- `Plan` adds no arguments. The supervisor owns the process; closing stdin (what the driver does on
  return) is how OpenCode ends, with exit code 0.

## Flavors

- OpenCode: environment `OPENCODE_CONFIG_CONTENT` with the edit ban and
  `OPENCODE_DISABLE_PROJECT_CONFIG=1`. After the session exists, an option with `category: "mode"`
  that offers `plan` and is not already `plan` is set to `plan` (also on resume). A login failure
  (`-32000`, or a message or data containing `auth_required`) is `NotLoggedIn`.
- Config dialect: the environment is fixed at spawn, so `plan` learns the CLI's major version
  before launch: the first word of the user's command is run as `<program> --version`
  (`probe_version`, never the whole command: `opencode acp --version` is wrong) on the network
  runtime while `plan` blocks on the result, for at most 5 s. Major 2 gets the 2.x array
  `{"permissions":[{"action":"edit","resource":"*","effect":"deny"}]}`; every other result, a
  failed probe included, gets the 1.x object `{"permission":{"edit":"deny"}}`, the only dialect
  checked. The probe is injectable (`AcpDriver::with_probe`). The first word of a wrapper or
  launcher (`npx`, a script) reports its own version; only a major of exactly 2 switches the
  dialect, so a launcher's version cannot select the 2.x form by accident.
- DeepSeek: `MISSING_CREDENTIAL` in the error message or data is `NotLoggedIn`; resume only
  through `session/resume`; the tool kind the agent reports is always `other`, so every permission
  request is allowed.
- Generic: no environment, no mode, resume by the declared capabilities, `auth_required` text is
  `NotLoggedIn`.

## Verified against OpenCode 1.19.4

Recorded in `fixtures/opencode/` (`*.jsonl` agent stdout, `*.sent.jsonl` what the client wrote);
the replay tests drive the real driver with them and assert the bytes it writes.

- `initialize`: `loadSession: true`, `sessionCapabilities: {close, fork, list, resume}`, no
  `systemPrompt`, `agentInfo` `OpenCode` `1.19.4`, `authMethods` with `opencode-login`.
  `session/new` returns `configOptions` `model` (values like `opencode/big-pickle`) and `mode`
  (`build`, `plan`); `session/resume` and `session/load` answer with `configOptions` and no
  `sessionId`. An `available_commands_update` follows `session/new`; thoughts and message text
  interleave.
- Config: `OPENCODE_CONFIG_CONTENT` accepts only the 1.x object. The 2.x array makes the process
  exit at startup ("V2 permissions are not supported by OpenCode V1").
- With `permission.edit: deny` the edit and write tools are absent from the toolset and no
  permission request arrives for shell commands: `echo hi > file` through `bash` succeeds. For a
  harness without a sandbox this is the accepted state: edit tools off, shell free, no
  confirmations. `bash: ask` is not used. Without the deny config the `write` tool runs with kind
  `edit`. In plan mode (`session/set_config_option` `mode` = `plan`, which answers with
  `configOptions`) the agent made no writes.
- Tool calls: `tool_call` has `title: "bash"`, `kind: "execute"`, `status: "pending"` and a
  `rawInput` with only the cwd; the command line is the `title` of the following
  `tool_call_update`s (`in_progress`, three to five times with the same content), then `completed`
  (kind absent). A `write` call keeps the title `write` while running and gets the file name on
  `completed`.
- A turn of five commands ends once: the `session/prompt` answer comes after the last text and a
  `usage_update`. Cancel: after `session/cancel` the tool completes and the answer
  `stopReason: "cancelled"` arrives within about 4 s; the next prompt in the same process works.
- `session/load` replays the history (`user_message_chunk`, `agent_message_chunk`, `tool_call`,
  …) before its answer; `session/resume` replays nothing, so it is the path used.
- An unknown session id fails `session/resume` and `session/load` with `-32603`, "Internal error:
  OpenCode service failure", `data: {"service": "session"}`.
- With the ask config a shell command sends `session/request_permission` with options `once`
  (`allow_once`), `always` (`allow_always`), `reject` (`reject_once`) and the tool call's kind
  `execute`.
- Closing stdin ends the process with exit code 0.

## Not run live

- DeepSeek (`dsh acp`) is built from its documentation only: no `session/load`, no
  `systemPrompt`, no modes, an empty `authMethods`, option ids `allow-once` / `reject-once`
  (the `kind` field is what is read), tool kind `other`. Whether its edits ask for permission at
  all is unknown; if they do not, the client cannot forbid them.
- Generic agents are covered by the spec and scripted peers only. The `systemPrompt` parameter
  name for the instruction text, the 2.x OpenCode dialect and the agents' possible deviations
  (an answer that never comes, a tool update without a call, a process that outlives its closed
  stdin) are handled by the rules above, not observed.

## Decisions

- Own `jsonrpc` and `serde_json::Value` instead of the `agent-client-protocol` crate: it brings a
  second I/O stack and rejects the deviations real agents make.
- A lost session fails the turn instead of continuing in a fresh one: the app owns the
  stored id and the re-sent brief, and hiding the loss would continue with an empty history.
- `ToolStarted` waits for the first non-pending status instead of re-emitting on a title change:
  the app's activity line shows the command once and a call is announced exactly once.
- Edits are refused by the agent's config and the permission rule; shell commands are allowed
  without confirmation.
- Not-yet-active turns drop updates: only a running turn's updates are events, which also
  discards a load replay without a flag.
- The OpenCode version is probed before launch and only a major of 2 selects the unverified
  dialect, so a failed or foreign probe falls back to the checked one.
