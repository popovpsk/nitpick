# review/agent

The review assistant: `AgentPanel`, a separate entity that `ReviewView` holds. It is the
slide-out side panel (frame, resize edge, conversation, input), the conversation of one review
loaded from and saved to `nitpick.db`, and the orchestration of the harness session behind it
(lazy start, overview, chat, recovery). The header button and the `ToggleAgent` action live in
`review/` and only read and drive this entity.

Not here: running the harness (`agent` crate), the profile table and settings
(`agent_profiles`, `settings_store`), persistence (`app_db`), the settings screen
(`settings_view`). `diff:` links, stops and the selection question come later and attach to the
seams at the end.

## Files

- `mod.rs` — `AgentPanel` (open / closing / width state, `Phase`, the unread flag, the CLI
  lookup, the conversation, the session), `Turn`, `Live`, `Activity`, `AgentEvent`,
  `apply_settings`, `locate` / `locate_in`, `unread_after`.
- `run.rs` — the orchestration: `set_review`, loading the conversation, `evaluate` (overview
  decision), `dispatch` / `dispatched` / `send_now`, the event pump and `apply_event`, failure
  recovery, persistence of messages, row bookkeeping, `init`, scratch folders, `retire`.
- `logic.rs` — the pure decisions: `OverviewState`, `load_action`, `Request`, `compose` /
  `compose_chat`, `classify` / `problem_text` / `exit_text`, `limit_text`, `coalesce`.
- `context.rs` — `ReviewContext` (key, head, the `Brief`, the work-directory plan), `from_view`
  (reads `ReviewView`), `review_key`, `local_head`, `assemble` (plain inputs to `Brief`),
  `resolve_workdir`, `language_name`.
- `messages.rs` — `Message`, `agent_markdown`, row rendering (`render_row`, the progress row).
- `panel.rs` — `frame` (the animated, resizable frame `ReviewView` places in the body) and
  `Render for AgentPanel` (title, outdated-overview banner, conversation or empty state,
  footer with the input).
- `tests.rs` — everything without a GPU: the program lookup against a throwaway `PATH`, phases,
  review keys, head fingerprint, brief assembly, work-directory choice on throwaway
  repositories, the overview decision table, failure texts, the changed-comments prefix,
  event coalescing.

## Invariants and spec

### Frame

- Non-modal and per window: the frame is drawn in `ReviewView`'s body next to the diff, which
  keeps taking input. Order left to right is diff, panel, tree when the tree is on the right;
  with the tree on the left the panel is last. Open state is not persisted, the tree's visibility
  is not either; the width is saved to `agent.panel_width` 400 ms after the last move by a
  detached task keyed on a static generation, so closing the window right after a drag still
  saves it.
- The slide copies the tree panel's: the content keeps its full width inside an
  `overflow_hidden` box and a closing frame stays drawn until `SLIDE` ends, covered so it
  takes no clicks; the frame exists only after a toggle, so it always animates. The drag listener
  sits on the frame itself, so the new width is the frame's right edge minus the pointer,
  whatever else shares the row.
- `ReviewView` observes the entity and re-renders on every `cx.notify()` of the panel; the panel
  forwards `AgentEvent::OpenSettings` and `ReviewView` re-emits it as `ReviewEvent::OpenSettings`.
- The CLI lookup resolves the active profile's command with `agent::process::Invocation`, the
  same lookup a launch uses, on `net_runtime::unblock` (the first call captures the login
  shell's environment and may take seconds). A path with a separator counts as found only when
  the file exists. It runs when the panel opens and when the active profile, its command or its
  model changes, never in `render`; a result of an older run is dropped by `lookup_generation`.
  A launch that reports "not found" sets the lookup to missing as well, so a panel that was
  never opened learns it.
- `render` only reads fields: the labels (`profile_name`, `missing_hint`) are rebuilt when
  settings change, since a language change arrives as a settings change too. Rows are built by
  `render_row` for the visible items only; nothing is collected per frame.
- `enabled` turned off closes the panel (animated) and `open` refuses to open it; the header
  button is built from `settings.agent.enabled` in `ReviewView`. Turning it off does not end a
  running session, and with it off nothing starts one by itself.
- `ToggleAgent` follows the other review actions: it propagates while an input has focus.

### Review and conversation

- `ReviewView::sync_agent` calls `set_review` with a `ReviewContext` after an install, a
  discussions load, a viewed-marks change and a hidden-files change. A review is identified by
  `(repo root, key)`: `mr:<forge>:<project>:<number>`, `branches:<target>..<source>`, `worktree`.
  A new identity ends the process (a running turn keeps its partial text), clears the list, loads
  the conversation from the database and only then decides about the overview. The same identity
  only replaces the context; a changed `head` re-evaluates the overview, an unchanged one does
  nothing, so an install with the same content (the `unchanged` fast path, a refresh) costs
  nothing here. A single-commit view yields no context, so the last whole-review context stays.
- `head` is the id the stored messages are tagged with: the head sha for a merge request and a
  branch comparison; for local changes `local:` plus an FNV-1a 64 hash over every file's path and
  blob-pair fingerprint, in file order, so it changes whenever the patch does.
- The `Brief` is rebuilt in full on every sync from plain inputs (`BriefInputs`); threads are
  ordered unresolved-first (a stable sort) because `brief::render_threads` drops later threads
  when the size limit is reached. `disk_matches_head` is filled when a prompt is built, never in
  the context.
- The message list is a `MessageScroller` (gpui's virtualized `list` with tail following, a
  scrollbar and a jump-to-latest button). Its row count is `messages + progress row`; `rows`
  mirrors it and `sync_rows`, `push_message` and `remove_message` are the only code that changes
  it, so an index never refers to a row the list does not have. A streaming message and a
  re-parsed one are remeasured (`remeasure_items`): the markdown view parses in the background and
  its height changes after the row was measured, which `observe` on each message's
  `TextViewState` catches (`row_changed`).
- Each message owns an `Entity<TextViewState>`, created when its row is first drawn (a long
  history costs nothing until the panel is opened, and a closed panel creates none). Streaming
  appends with `push_str` and the view fades the new words in. Assistant, user and notice rows are
  different: notices are plain muted text, user messages sit in a tinted box under a "You" label.
- Persistence: the user's message and notices are written when they are made, the assistant's
  once when its turn ends (completed, interrupted, failed or the process died); text that is
  only whitespace is not stored and its row is removed. A stopped turn keeps its partial text and
  gets a notice. Writes are queued on the database thread in call order, so a clear followed by a
  read never sees old rows.

### Session

- Lazy: nothing starts until a prompt must be sent. `evaluate` starts the overview right after
  the conversation loads when `agent.auto_overview` and `agent.enabled` are on, even with the
  panel closed; otherwise the first button press or message does. `dispatch` is the only way in:
  it resolves the work directory on the background executor (`context::resolve_workdir`), reads
  the stored session id of `(repo, key, profile)` from the database, calls `Session::start` on
  `net_runtime::unblock` (the first call waits for the login-shell snapshot), and comes back to
  `dispatched`, which discards a result whose `operation` is no longer current (a stop, a review
  change or a settings change in between). A chat message on a live session whose brief was sent
  skips all of that and goes out at once.
- Work directory: local changes use the repository root. Branches and merge requests use the
  source branch's own checkout (`checkout_of`), else the root. `disk_matches_head` is whether
  that directory's `HEAD` equals the reviewed head, recomputed for every prompt that carries a
  brief against the live session's directory; it only reads (`git worktree list`, `rev-parse`).
  Local changes always count as matching.
- The brief goes with a prompt when the session is new (no stored id, or a lost one) and always
  with an overview or update. A resumed session already has one; its chat messages instead get
  the "new and changed comments" prefix for threads whose fingerprint differs from the ones sent
  with the last prompt (`seen`). `seen` is not persisted, so the first message after the app was
  restarted resends all threads once.
- Overview decision (`logic::load_action`): no stored overview and auto on → overview; stored for
  the current head → show only; stored for another head → show it under a banner with an "Update
  the overview" button, and with auto on and a non-local review run the update (its task carries
  the old and the new head). Local changes are reloaded on every window activation, so their
  overview is only ever updated by the button. An action runs at most once per head
  (`auto_attempted`), and a head change during a turn is evaluated when the turn ends (`stale`).
- Events come through `flume::Receiver::recv_async` in a task owned by `Live`. After a text or
  thinking event the pump waits one frame (16 ms), drains the channel and merges adjacent text
  and thinking events (`logic::coalesce`), so the UI is updated at most once per frame during a
  burst. Every batch is checked against `generation`, which changes whenever a session is
  retired; a batch of a superseded session is dropped and its pump ends. Dropping `Live`
  cancels the pump.
- Event handling: `Session` stores the id per profile; `Text` creates or extends the turn's
  assistant message; `ToolStarted` / `ToolFinished` / `Thinking` / `Notice` only change the
  progress row (never stored); `TurnFinished` ends the turn; `Exited` is last and retires the
  session. A turn that ends in `SessionLost` clears the stored id, retires the session, adds a
  notice and sends the same request again once (`Turn::retried`); the old session's `Exited` is
  never seen because the generation moved. Any other failure becomes `Phase::Failed` with a
  localized text; an `Exited` during a turn becomes a failure too and leaves the panel ready for
  a new session that resumes the stored id.
- While a turn runs the send button is a stop button (`Session::interrupt`; while still starting
  it cancels the start) and Enter sends nothing: the input stays editable for a draft and a line
  under it says why. Enter sends, Shift+Enter inserts a newline (`submit_on_enter` on the
  textarea; the `PressEnter` of a Shift+Enter is ignored).
- A changed profile, command or model ends the process and adds a notice; the conversation
  stays, and session ids stay per profile. Closing the window (`on_release`), a review change and
  "New conversation" end it too. Ending is `retire`: `Session::shutdown`, then the session's scratch
  folder is removed; the scratch folders of a killed app are swept at the next start when older
  than a day.
- "New conversation" (title row) deletes the review's messages and every profile's session id
  and does not start an overview by itself; it is also the way out of `ContextTooLong`.
- `agent::process::prewarm()`, the history pruning (`prune_agent`, once at startup like the
  avatar cache's) and the scratch sweep run from `init`, called from `main`.

## Seams for the next stage

- `AgentPanel::send_chat(shown, prompt, cx)` is the programmatic submit: `shown` is what the
  conversation and the database get, `prompt` what the agent gets (before the brief or comments
  prefix). The input is the `input: Entity<TextareaState>` field (`set_value`, `focus`).
- `agent_markdown` is the one place where assistant and user markdown gets its link handler;
  it opens only http, https and mailto and ignores everything else.
- `Message` keeps `text` (the markdown source) for every message, which is where links are
  extracted from; `apply_event`'s `TurnFinished` arm is where a finished overview can be scanned.
- `ReviewContext::brief.files` lists the review's files for checking that a link names one.

## Decisions

- The panel is a non-modal frame like the file tree, not a sheet like the comments, so a link
  can move the diff while the panel stays open.
- Open state is per window and never saved; only the width is a setting.
- The overview button stays visible but disabled until a session can start, rather than being
  absent: the empty state would otherwise invite something with no way to do it.
- Default key for `ToggleAgent` is ⌘/Ctrl-Shift-A (free, "assistant").
- A message typed during a running turn is not queued: sending is disabled and the panel says so.
  A queue would need a second, invisible turn state that could not be stopped or reordered.
- The list uses `MessageScroller` (top alignment, tail following) instead of a bare `list` with
  bottom alignment: it is gpui-component's own chat widget with the follow-the-tail and
  scrolled-away behaviour already decided.
- The context is pushed by `ReviewView` instead of pulled by the panel; see `review/doc.md`.
- A stopped turn keeps its partial text and a notice says so, rather than marking the text
  itself, so the stored answer is only what the agent wrote.
- The request for an overview or an update has no user message in the conversation (its text is
  the whole brief); the assistant's answer is the first thing the user sees.
- Profile notices and failures are stored as notices or shown as a banner; a failure is not
  stored, since it describes the moment and the next action replaces it.
