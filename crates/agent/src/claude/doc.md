# agent::claude

The Claude Code driver: builds the command line, `settings.json` and instructions file for a
read-only, non-interactive session, runs the turn loop over the CLI's bidirectional stream-json
pipes and turns its frames into `Event`s. GPUI-free.

Not here: starting or killing the process (`session`, `process`), the instruction text
(`Launch.instructions` comes from the app).

## Files

- `mod.rs` — `ClaudeDriver` (`Driver::plan` and `run`), the pure builders `arguments`,
  `settings_json`, `environment`, `Os`, `SessionArg`, the session-file lookup (`session_file`),
  and the turn loop.
- `history.rs` — `read` / `parse`: the session file to `Entry`s.
- `wire.rs` — frame types (every field optional, a mistyped field reads as absent) and the lines
  written to stdin.
- `normalize.rs` — `Normalizer`: frames to `Event`s, tool titles, result classification.

## Invariants and spec

- Command line, after the user's command: `--print --input-format stream-json --output-format
  stream-json --verbose --include-partial-messages --tools Bash,Read,Grep,Glob --allowedTools
  Bash,Read,Grep,Glob --disallowedTools Agent,Task,Workflow,AskUserQuestion,ExitPlanMode
  --permission-mode dontAsk --setting-sources user --strict-mcp-config --disable-slash-commands
  --settings <scratch>/settings.json --append-system-prompt-file <scratch>/instructions.md
  [--model <m>]` and then `--session-id <uuid>` or `--resume=<id>`. Every flag repeats on resume:
  none is restored from the session, except the system prompt (below). Not `--bare` (it breaks subscription login),
  `--permission-prompt-tool`, plan mode.
- Environment added: `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY`,
  `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`, all `1`.
- `settings.json` is `{"disableAllHooks":true,"sandbox":{"enabled":true,
  "allowUnsandboxedCommands":false,"filesystem":{"denyWrite":[<absolute cwd>]}}}`; on Windows there is no `sandbox` block (no sandbox there).
  `denyWrite` on the cwd is required: without it a shell write inside the cwd succeeds; with it
  the write fails with "operation not permitted". Unsandboxed retries (the Bash tool's
  `dangerouslyDisableSandbox`) are switched off by `allowUnsandboxedCommands: false`, so the model
  does not attempt them; on 2.1.330 `dontAsk` mode also refuses such a retry and lists it in
  `permission_denials`, which is a second line of defence, not the one relied on.
- Windows argv holds only words and paths of files in `scratch`: instructions and settings never
  go into arguments. The platform is a parameter (`Os`) of the builders so that shape is tested on
  every OS.
- The system prompt is stored with the session at its first request and a resume keeps it, even
  with the file changed between launches (2.1.330: a session created with "the codeword is
  ALPHA", resumed with "OMEGA", answers ALPHA), so context that changes goes in messages, not in
  the instructions file. The app does exactly that for the instructions of a resumed session
  (`review/agent/doc.md`); the file written on resume matters only for the next new session.
- Facts about CLI 2.1.330 the code relies on:
  - `init` lists `tools: ["Bash","Glob","Grep","Read"]` with these flags.
  - The repository's `CLAUDE.md` is not loaded under `--setting-sources user`.
  - `system/init` is re-sent at the start of every turn: `Event::Session` is emitted when the id
    differs from the last one reported.
  - Thinking arrives as an empty `thinking` block start plus `system/thinking_tokens` frames; the
    `thinking_delta` text is usually empty, so `Event::Thinking` rarely fires. Empty deltas are
    never forwarded. Unknown `system` subtypes (`status`, `thinking_tokens`) are ignored.
  - Text arrives as `stream_event` deltas and again whole in the `assistant` frame: the whole
    text is used only when no delta of that message was streamed (the flag resets at
    `message_start`). Thinking follows the same rule.
  - `tool_use` is read from the `assistant` frame (the stream's block start has an empty
    `input`), so a title is available only once the input is complete. A tool without a
    `tool_result` when the turn ends is closed as failed.
  - Frames with a non-empty `parent_tool_use_id` belong to a sub-agent and are ignored.
  - Interrupt: the CLI answers a `control_response`, then a `user` frame with an `is_error`
    `tool_result`, then `result` with `subtype: error_during_execution`, `is_error: true`,
    `terminal_reason: aborted_tools` (or `aborted_streaming`) and `result: null`. That is
    `Outcome::Interrupted`, decided by `terminal_reason` alone. The process stays alive and takes
    the next prompt.
  - Missing session on `--resume`: one `result` frame (`error_during_execution`, `is_error`,
    `result: null`, `errors: ["No conversation found with session ID: <id>"]`), the same text on
    stderr, exit code 1. The message is in `errors[]`, so failure text is `errors`, else
    `result`, else the text of an errored `assistant` frame, else a generic line.
  - `rate_limit_event` with `status: "allowed"` is normal (`overageStatus` may say `rejected`
    in it). Only `status == "rejected"` without `isUsingOverage` marks a limit, and only
    matters when the turn then fails.
- Classification uses codes first and text as a hint: `SessionLost` (text "No conversation found
  with session ID"), `NotLoggedIn` (401, `authentication_failed`, "not logged in", "please run
  /login", "invalid api key"), `UsageLimit` (429, `rate_limit`, a rejected limit event),
  `ContextTooLong` (`terminal_reason` `prompt_too_long` or `blocking_limit`, "prompt is too
  long"), `ModelNotFound` (`model_not_found`, "issue with the selected model"), else `Other`. An
  API error arrives as `subtype: success` with `is_error: true`; any `subtype` other than
  `success` also fails. Text of an errored `assistant` frame is never shown as an answer.
- Every `result` ends the turn, even one that arrives while no prompt is outstanding (a missing
  session can answer before the prompt is written). An unreadable line that contains
  `"type":"result"` ends it too, as `Failed(Other)`. Any other bad line is logged (first 200
  characters) and skipped.
- Prompts: one turn at a time. A `Prompt` during a turn is queued and written after that turn's
  `result`. `Interrupt` writes a `control_request` only while a turn runs; queued prompts
  survive it and go out after the interrupted `result`.
- End of stdout (or a failed stdin write) with a turn running fails that turn and each queued
  prompt, one `TurnFinished(Failed(Other))` each, with open tools closed as failed, so the UI never
  waits; the supervisor then adds `Exited`. Prompts already sitting in the command channel at that
  moment count as queued: the loop picks between a ready command and a ready EOF at random, so
  without the drain a submitted prompt could vanish without its `TurnFinished`. On `Shutdown` the driver returns without events.
- `can_use_tool` requests are denied; any other `control_request` is answered with an error so
  the CLI never waits for a reply.
- `--resume` is used only after the session file `<config>/projects/*/<id>.jsonl` is found
  (`<config>` is `CLAUDE_CONFIG_DIR` as the child will see it, else `~/.claude`; the id may hold
  only letters, digits, `-`, `_`). Otherwise `plan` returns `Plan::refusal` and nothing is
  launched. A wrapper command that points the CLI elsewhere is not known to the check.
- The lookup is injectable (`ClaudeDriver::with_lookup`, it returns the file's path) so tests
  never read the home folder.
- History: the CLI has no command that prints a session, so with `Launch.history` the driver
  reads the file the lookup found. `plan` keeps its path in the driver (one driver per session)
  and `run` reads it on `net_runtime::unblock` before reading stdout, so `History` comes before
  the first turn's `Session`. Files over 64 MiB give no history. Facts about 2.1.331–2.1.332:
  one JSON record per line, of many types (`user`, `assistant`, `attachment`,
  `queue-operation`, `api-request-blob`, `system`, …); a prompt is a `user` record whose
  `message.content` is the string as sent; every content block of an answer is its own
  `assistant` record carrying `message.id`; tool output is a `user` record of `tool_result`
  blocks. Read: `user` and `assistant` records not marked `isSidechain`, `isMeta`,
  `isCompactSummary` or `isApiErrorMessage`, without a string `error` and not by the
  `<synthetic>` model; a prompt is its string or `text` blocks, `[Request interrupted by user…`
  is dropped; the `text` blocks of the answers until the next prompt make one entry (blocks of
  one message joined directly, messages by a blank line, as the live stream reads). Records
  are taken in file order; the `parentUuid` chain is not followed, since a print-mode session
  never branches.

## Model list

- `catalog::claude_models`: the same flags as a session (`plan`, in an empty scratch folder,
  never the user's repository), then one `{"type":"control_request","request_id":"init-1",
  "request":{"subtype":"initialize"}}` on stdin and no prompt. The reply (about 0.3 s, no model
  call; live 2.1.330) is the `control_response` whose `request_id` matches, after
  `system/hook_*` frames; `response.response.models[]` has `value` (the id passed back with
  `--model`), `displayName`, `description` and `resolvedModel`. The `default` entry is not
  listed: the entry whose `resolvedModel` equals its own is marked `is_default`. The same reply
  carries an `account` object with the user's email: only `models` is read, nothing is logged
  or kept, and the fixture `fixtures/claude/models.jsonl` keeps only `models` and the version.

## Known limits

- User-level sandbox settings from `~/.claude/settings.json` (for example
  `sandbox.excludedCommands`) still merge in under `--setting-sources user`: a command the user
  excluded from their own sandbox runs unsandboxed here too. That is the user's configuration and
  is left alone.

## Decisions

- A missing session is delivered through the event stream (`Plan::refusal`, see the crate doc),
  not as an `Error`: the app's contract for it is `FailureKind::SessionLost`.
- No empty `Event::Thinking` marker for "the model is thinking": the app decides how to show
  progress from `TurnFinished` and tool events.
- Queued prompts survive an interrupt.
