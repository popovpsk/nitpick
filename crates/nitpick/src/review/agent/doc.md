# review/agent

The review assistant: `AgentPanel`, a separate entity that `ReviewView` holds. It is the
slide-out side panel (frame, resize edge, conversation, input), the conversation of one window
(in memory only, never written to `nitpick.db`), and the orchestration of the harness session
behind it (lazy start, overview, chat, resume, recovery). The header button and the `ToggleAgent` action live in
`review/` and only read and drive this entity.

Not here: running the harness (`agent` crate), the profile table and settings
(`agent_profiles`, `settings_store`), the stored session pointer (`app_db`), the settings screen
(`settings_view`), the link grammar (`review_core::diff_link`) and the selection question text
(`agent::prompt`). Following a link moves the diff, so that part is `ReviewView::open_diff_link`
(`review/`); this module decides what a link means and keeps the pending selection.

## Files

- `mod.rs` — `AgentPanel` (open / closing / width state, `Phase`, the unread flag, the CLI
  lookup, the conversation, the session binding), `Turn`, `Live`, `Bound`, `Activity`,
  `AgentEvent`, `apply_settings`, `locate` / `locate_in`, `unread_after`.
- `run.rs` — the orchestration: `set_review`, reading the stored session pointer, `evaluate`
  (overview decision), `dispatch` / `dispatched` / `send_now`, the event pump and
  `apply_event`, session binding and its stored state, failure recovery, row bookkeeping,
  `init`, scratch folders, `retire`.
- `claims.rs` — the process-wide set of session ids held by a window (`Claim`, `is_held`,
  `next_owner`).
- `logic.rs` — the pure decisions: `OverviewState`, `load_action`, `Request`, `compose` /
  `compose_chat`, `goes_out_at_once`, `adopt`, `Resume`, `instructions_due`,
  `with_updated_instructions`, `Launching`, `classify` / `problem_text` / `exit_text`,
  `limit_text`, `coalesce`.
- `context.rs` — `ReviewContext` (key, head, the `Brief`, the work-directory plan), `from_view`
  (reads `ReviewView`), `review_key`, `local_head`, `assemble` (plain inputs to `Brief`),
  `resolve_workdir`, `language_name`.
- `messages.rs` — `Role`, `Kind`, `Message`, `agent_markdown` (the link
  hook and click handler), row rendering (`render_row`, the progress row).
- `links.rs` — everything about `diff:` links without a GPU: `classify`, `resolve_file`,
  `plan_link` (what a click does), the selection question (`selection_span`,
  `can_link_selection`, `patch_sides` / `locate_selection` for a mouse text selection,
  `PendingSelection`, `compose_question`, `input_with_marker`); on `AgentPanel`: `ask`.
- `panel.rs` — `frame` (the animated, resizable frame `ReviewView` places in the body) and
  `Render for AgentPanel` (title with "New conversation", outdated-overview banner, conversation
  or empty state, footer with the input).
- `tests.rs` — everything without a GPU: the program lookup against a throwaway `PATH`, phases,
  review keys, head fingerprint, brief assembly, work-directory choice on throwaway
  repositories, the overview decision table, failure texts, the changed-comments prefix,
  session claims and adoption, the chat-after-overview route, instruction delivery on resume,
  launch comparison, event coalescing, link resolution against a line map, the
  selection question and locating a text selection in a patch.

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
  forwards `AgentEvent::OpenSettings` and `ReviewView` re-emits it as `ReviewEvent::OpenAssistantSettings`,
  which opens the settings window on its Assistant page.
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
  A new identity ends the process (a running turn keeps its partial text), clears the list and
  the window's session binding, reads the stored session pointer for the active profile and only
  then decides about the overview. The same identity
  only replaces the context; a changed `head` re-evaluates the overview, an unchanged one does
  nothing, so an install with the same content (the `unchanged` fast path, a refresh) costs
  nothing here. A single-commit view yields no context, so the last whole-review context stays.
- `head` is the id the messages are tagged with: the head sha for a merge request and a
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
- The conversation lives only in the window's memory: messages, overview and notices
  are never written to `nitpick.db` or read from it, and are gone when the window closes or
  the review changes. Text that is only whitespace is dropped and its row removed when its turn
  ends (completed, interrupted, failed or the process died). A stopped turn keeps its partial
  text and gets a notice. What survives is the harness's own session, which a stored pointer
  can continue (see Session).
- A window with no messages but a free stored session id for the active profile (after a
  restart, or on returning to the review) is resumable: it shows "continue the earlier
  conversation" instead of the overview invitation, the input stays enabled, and no automatic
  overview starts (`load_action` with `resumable`), since it would spend a full overview on
  every app start. "Request an overview" stays on screen and runs as a turn of that session.

### Session

- Lazy: nothing starts until a prompt must be sent. `evaluate` starts the overview right after
  the conversation loads when `agent.auto_overview` and `agent.enabled` are on, even with the
  panel closed; otherwise the first button press or message does. `dispatch` is the only way in:
  it resolves the work directory on the background executor (`context::resolve_workdir`), picks
  the session to resume (below), calls `Session::start` on
  `net_runtime::unblock` (the first call waits for the login-shell snapshot), and comes back to
  `dispatched`, which discards a result whose `operation` is no longer current (a stop, a review
  change or a settings change in between). `dispatched` starts a session itself
  (one more `dispatch`, which then has none to lose) when the request found a live session at
  dispatch time and that session ended while the work directory was being resolved. A chat message on a live session whose brief was
  delivered skips all of that and goes out at once.
- Work directory: local changes use the repository root. Branches and merge requests use the
  source branch's own checkout (`checkout_of`), else the root. `disk_matches_head` is whether
  that directory's `HEAD` equals the reviewed head, recomputed for every prompt that carries a
  brief against the live session's directory; it only reads (`git worktree list`, `rev-parse`).
  Local changes always count as matching.
- The brief goes with a prompt until it counts as delivered, and always with an overview or
  update. It is delivered when a turn that carried it ends `Completed` or `Interrupted`
  (`logic::brief_delivered`); a failed turn leaves it pending, so the next request carries it
  again even on the same live session. A resumed session counts as having it only when the
  stored `brief_delivered` flag of that session id says so (set when a turn that carried the
  brief was answered, reset with a new id, deleted with the row): a stored id alone proves
  nothing, since harnesses report the id before any prompt, and the window may have no
  messages to judge by. The flag is per profile because the row is. Once delivered, chat
  messages get the "new and changed comments" prefix for threads whose fingerprint differs from
  the ones in the last answered turn (`seen`, updated with the delivery, so a failed turn does
  not mark its threads as seen). `seen` is not persisted, so the first message after the app was
  restarted resends all threads once.
- One conversation, one session: the overview, an update and every later question are turns of
  the same harness session. A chat on a live process whose brief was delivered goes out at once
  (`logic::goes_out_at_once`); if the process is gone, the next request resumes the same session
  id. A window uses the id it is bound to (`Bound`, held in memory) and reads the stored
  pointer only when it has none.
- Session claims: a session id is used by at most one window at a time. A window holds a
  `claims::Claim` for the id it is bound to, taken when it adopts the stored id
  (`logic::adopt`, inside the dispatch task, before the process starts) or when `Event::Session`
  reports a new id, and released when the binding ends: another review, "New conversation", a
  lost session, another profile, or the window closing (`Claim` drops with its owner). A claim
  survives the process: a session that ended or was restarted for new settings is resumed by
  the window that owns it, and no other window can pick its id up from the stored row in the
  meantime. A window that would adopt an id another window holds starts a new session
  instead. The stored row is last-writer-wins (`set_agent_session`), and a window that loses
  its session clears the row only if it still names that id.
- The system prompt is not always applied on resume (Claude Code keeps the one the session was
  created with and ignores a new `--append-system-prompt-file`; ACP agents without
  `systemPrompt`, and possibly Pi, never take instructions on resume). So the stored row
  carries the fingerprint of the instructions the session last received
  (`prompt::instructions_fingerprint`); when a resumed session's fingerprint differs from the
  text built for this launch (the user's text, the language or the disk rule changed), the
  full text goes in front of the first message as an "instructions were updated" block
  (`prompt::updated_instructions`) and the fingerprint is stored once that turn is answered; a
  failed or cancelled turn leaves it due for the next message.
- Overview decision (`logic::load_action`): no overview in the window and auto on → overview
  (not when the window is resumable); written for the current head → show only; for another head → show it under a banner with an "Update
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
- Event handling: `Session` binds the window to the id (claim, then the stored pointer per
  profile); `Text` creates or extends the turn's
  assistant message; `ToolStarted` / `ToolFinished` / `Thinking` / `Notice` only change the
  progress row; `TurnFinished` ends the turn; `Exited` is last and retires the
  session. A turn that ends in `SessionLost` clears the binding and the stored id, retires the session, adds a
  notice and sends the same request again once (`Turn::retried`); the old session's `Exited` is
  never seen because the generation moved; a `SessionLost` with no turn running (a failed
  setup after a cancel) only clears the id and shows the failure, with no notice and no re-send.
  A turn that ends `Cancelled` (the prompt never reached the harness) shows nothing
  new and the phase returns to `Idle`: the user's question stays in the list, no stopped notice
  is added (nothing was answered, so "partial" would be false), the brief and the comments stay
  pending (`logic::answered` is false), and a cancelled overview or update pins `auto_attempted`
  to the turn's head, so with `auto_overview` on it does not restart until the head, the
  settings or an explicit request change; a Stop while the session is still starting goes
  through the same `auto_attempted_after` with the request that was starting, and then evaluates
  a head change seen meanwhile, so both stop paths end alike: a stopped overview or update is
  not retried for its head, a stopped question pins nothing, and a newer head is still handled. A cancelled question's pending selection is already cleared at
  send, so its marker must be asked for again (`AskAgent`); the link stays in the message.
  Any other failure becomes `Phase::Failed` with a localized text; an `Exited` during a turn becomes a failure too and leaves the panel ready for
  a new session that resumes the window's session id.
- While a turn runs the send button is a stop button (`Session::interrupt`; while still starting
  it cancels the start) and Enter sends nothing: the input stays editable for a draft and a line
  under it says why. Enter sends, Shift+Enter inserts a newline (`submit_on_enter` on the
  textarea; the `PressEnter` of a Shift+Enter is ignored).
- A changed profile, command, model or the user's instructions (compared trimmed,
  `logic::Launching`) ends the process and the next session uses the new settings; the
  conversation stays. Session ids stay per profile: another profile ends the window's binding
  (and its claim), the same profile with another command, model or instructions resumes its
  session, which gets changed instructions through the update block above. A notice is added only when something ran under the previous settings (a live or
  starting session, or a dispatch since the last change) and the conversation is not empty, and
  never right after another settings notice (`logic::settings_notice`), so saving a half-typed
  command several times leaves at most one. A change also makes the review's automatic overview
  eligible again at the next evaluation, without starting anything by itself.
- Closing the window, a review change and "New conversation" end the session too (and release
  the window's claim). Ending is `retire`: `Session::shutdown`, then the session's scratch folder is removed; `Live`
  retires on drop, and a started session is held in a `logic::Guard` from the moment it exists
  on the blocking thread, so a start whose dispatch was cancelled while `Session::start` ran is
  retired when the result is dropped, wherever that happens. The scratch folders of a killed
  app are swept at the next start when older than a day. A failed or refused start removes its
  own folder in the `agent` crate.
- With `auto_overview` on and a missing CLI, a review makes one start attempt per head and
  window (`auto_attempted`); a "not found" sets the lookup to missing, which blocks further
  automatic attempts until the settings change or the panel is opened (which re-checks).
- "New conversation" (title row) clears the window's messages and binding and deletes every
  profile's stored session id for the review; it does not start an overview by itself; it is
  also the way out of `ContextTooLong`.
- `agent::process::prewarm()`, the pruning of stale stored session ids (`prune_agent`, once at
  startup like the avatar cache's) and the scratch sweep run from `init`, called from `main`.

### Links

- `agent_markdown` is the one place where assistant and user markdown gets its link handling.
  gpui-base hands the link target to `TextView::on_link_click` exactly as written (the parser
  keeps `diff:` destinations and percent escapes untouched), and without a handler it would call
  `App::open_url` for any scheme, so the handler is always set. `diff:` links are dispatched as
  `OpenDiffLink { url }` with `window.dispatch_action` (the callback must be `Send + Sync`, and
  the action is dispatched on the focused element and travels up to `ReviewView`'s root element,
  which is why the handler does not check for a focused input: a click is not typing). The `link` hook draws every link that is neither `diff:` nor
  http, https or mailto as plain, unclickable text; the check is a prefix test, since the hook
  runs on every draw. Only a plain left click follows a `diff:` link; a middle click still opens
  web links.
- A `diff:` URL is model output, so it can only select a file of the open review and scroll:
  `diff_link::parse` rejects anything that is not the grammar, the path is looked up among the
  review's files and never touched on disk, and nothing is handed to the OS opener.
- `plan_link` is the whole decision of a click: file found (current path, then `old_path`, the
  current name winning over another file's old name) or not in the review; the scroll target is
  the first line of the range that the file's shown line map has, on the side the link names
  (`L` links scroll the old side); no such line keeps the view on the file. `ReviewView` applies
  it: see `review/doc.md` for the single-commit view, the interdiff and pending content.

### Question about a selection

- `AskAgent` opens the panel, puts the marker (`[path R10-12](diff:…)`, the very link
  `prompt::selection_link` writes) into the input and focuses it. It is reached by the key
  and by "Ask the assistant" in the context menu of the diff area (shown only while
  `agent.enabled`; the diff had no context menu before, and the file name's menu is another
  one). With no selection it only opens the panel and focuses the input.
- Where the selection comes from: a gutter (line number) gesture gives a line range directly
  (`DiffState::selected_lines`). An ordinary mouse text selection clears that range and the
  diff exposes only its text (`selected_text`), so file, side and lines are found by locating
  that text in the shown file's patch section (`patch_sides` splits it into an old and a new
  side with line numbers, `locate_selection` finds the text as a substring across line breaks).
  The file is the one shown. Several matches (a repeated `}`) are resolved by the hovered line
  (`ReviewView::hovered_line`, captured when the context menu opens): same side first, then
  the nearest line; with no hover the first match wins, new side before old. Limits: the
  text is matched, not the pointer, so a repeated text can link another occurrence (the agent
  still gets the selected text in full); a selection that crosses from one side to the other
  in the unified view has no text from the diff and so only opens the panel; the search reads
  the patch, so it sees what the patch shows (whole file with full context, else the hunks);
  nothing is derived while an interdiff is shown. A second ask while the old marker is still in the input
  replaces it, so the input never holds two markers for one pending selection.
- The pending selection (`pending`: path, side, range, the selected text clipped to 20 000
  characters, and the marker string) lives in the panel until the next send, a new ask, "New
  conversation" or another review. At send time `compose_question` checks whether the marker is
  still in the typed text: if so, `shown` is the text as typed (the marker renders as a link in
  the conversation) and `prompt` is `prompt::selection_question` over the text with the marker
  removed (`without_marker`: only the seam it leaves is tidied, so lines, lists and indented code
  keep their shape), so the agent sees the link once, with the selected text in a code block; if the user
  deleted or edited the marker, the message goes out as plain text and nothing of the selection
  is sent. A marker with no other text sends the selection alone.
- A selection that spans both sides of a unified view cannot be one `diff:` range, so the link
  covers the line the selection ended on, like adding a comment does; the agent still gets the
  whole selected text. The selection is not linked in the single-commit view, whose line numbers
  are another version's, or on the old side of an interdiff, whose old version is not the
  review's; the panel just opens then.
- A disabled assistant (`agent.enabled` off) makes `AskAgent` do nothing.

### The user's instructions

- `agent.instructions` (settings) is appended by `agent::prompt::instructions` after the
  built-in text under "Additional instructions from the user (the rules above still apply)",
  for every profile; trimmed, cut at `MAX_USER_INSTRUCTIONS` (20 000 characters), nothing added
  when blank. It is the user's own text and is not fenced as untrusted. It cannot turn off the
  read-only setup: that is enforced by the launch flags and the sandbox of each driver, not by
  the prompt, which a user text can only repeat or contradict.
- It is part of `Launching`, so a change ends the running session like a changed model; delivery
  on a resumed session is described under Session.

## Seams

- `AgentPanel::send_chat(shown, prompt, cx)` is the programmatic submit: `shown` is what the
  conversation gets, `prompt` what the agent gets (before the brief or comments
  prefix). The input is the `input: Entity<TextareaState>` field.
- `Message` keeps `text` (the markdown source) for every message.
- `ReviewContext::brief.files` lists the review's files.

## Decisions

- The panel is a non-modal frame like the file tree, not a sheet like the comments, so a link
  can move the diff while the panel stays open.
- Open state is per window and never saved; only the width is a setting.
- The overview button stays visible but disabled until a session can start, rather than being
  absent: the empty state would otherwise invite something with no way to do it.
- Default key for `ToggleAgent` is ⌘/Ctrl-Shift-A (free, "assistant"). `AskAgent` is
  ⌘/Ctrl-Shift-K, both avoiding bracket keys, which sit on letters in the Russian layout. Both
  propagate while an input has focus, like the other review actions; `OpenDiffLink` is a click
  and is not in `COMMANDS` or bindable.
- The owner removed the walk through an overview's links (a counter with previous and next
  buttons and two keys): the links in the text are the navigation, and a click on one is all
  that follows a `diff:` link.
- A `diff:` click follows the link even when the model's numbers are wrong: a line that is not
  in the patch leaves the view on the file instead of reporting an error.
- A message typed during a running turn is not queued: sending is disabled and the panel says so.
  A queue would need a second, invisible turn state that could not be stopped or reordered.
- The list uses `MessageScroller` (top alignment, tail following) instead of a bare `list` with
  bottom alignment: it is gpui-component's own chat widget with the follow-the-tail and
  scrolled-away behaviour already decided.
- The context is pushed by `ReviewView` instead of pulled by the panel; see `review/doc.md`.
- A stopped turn keeps its partial text and a notice says so, rather than marking the text
  itself, so the answer is only what the agent wrote.
- The request for an overview or an update has no user message in the conversation (its text is
  the whole brief); the assistant's answer is the first thing the user sees.
- Profile notices are messages of the window and failures are shown as a banner; a failure is
  not kept, since it describes the moment and the next action replaces it.
- A conversation belongs to its window and to the harness, not to nitpick's database: each
  window has an independent thread and threads never intersect. Nitpick stores only the id
  (and two flags) to continue a harness session from, so a thread survives a restart as the
  agent's own history, not as messages the app can show; the owner chose this over a shared,
  stored transcript.
- An earlier conversation is resumed on the first message, not at startup: an automatic
  overview would spend a full overview on every app start.
- Changed instructions on a resumed session travel as a message block, for every harness,
  rather than relying on each driver's handling of its system-prompt flag on resume.
