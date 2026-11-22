# review/agent

The review assistant's side panel: `AgentPanel`, a separate entity that `ReviewView` holds, with
the slide-out frame, the drag-to-resize edge, the title row and the states the panel shows
before a session exists (the overview invitation, the missing CLI). The header button and the
`ToggleAgent` action live in `review/` and only read and drive this entity.

Not here: running the harness (`agent` crate), the profile table and settings (`agent_profiles`,
`settings_store`), persistence (`app_db`), the settings screen (`settings_view`). Session
orchestration, messages and chat input, `diff:` links and stops come later and attach to the
seams below.

## Files

- `mod.rs` — `AgentPanel` (open / closing / width state, `Phase`, the unread flag, the CLI
  lookup), `AgentEvent::OpenSettings`, `locate` / `locate_in`, `unread_after`.
- `panel.rs` — `frame` (the animated, resizable frame `ReviewView` places in the body) and
  `Render for AgentPanel` (the contents).
- `tests.rs` — program lookup against a throwaway `PATH`, `Phase::is_busy`, `unread_after`.

## Invariants and spec

- Non-modal and per window: the frame is drawn in `ReviewView`'s body next to the diff, which
  keeps taking input. Order left to right is diff, panel, tree when the tree is on the right;
  with the tree on the left the panel is last. Open state is not persisted, the tree's visibility
  is not either; the width is saved to `agent.panel_width` 400 ms after the last move by a
  detached task keyed on a static generation, so closing the window right after a drag still
  saves it.
- The slide copies the tree panel's: the content keeps its full width inside an
  `overflow_hidden` box and a closing frame stays drawn until `SLIDE` ends, covered so it
  takes no clicks; the frame exists only after a toggle, so it always animates. The drag listener sits on the frame itself, so the new width is the frame's right
  edge minus the pointer, whatever else shares the row.
- `ReviewView` observes the entity and re-renders on every `cx.notify()` of the panel; the panel
  forwards `AgentEvent::OpenSettings` and `ReviewView` re-emits it as `ReviewEvent::OpenSettings`.
- The CLI lookup resolves the active profile's command with `agent::process::Invocation`, the
  same lookup a launch uses, on `net_runtime::unblock` (the first call captures the login
  shell's environment and may take seconds). A path with a separator counts as found only when
  the file exists. It runs when the panel opens and when the active profile or its command
  changes, never in `render`; a result of an older run is dropped by `lookup_generation`.
- `render` only reads fields: the labels (`profile_name`, `missing_hint`) are rebuilt when
  settings change, since a language change arrives as a settings change too.
- `enabled` turned off closes the panel (animated) and `open` refuses to open it; the header
  button is built from `settings.agent.enabled` in `ReviewView`.
- `ToggleAgent` follows the other review actions: it propagates while an input has focus.
  Nothing in the panel takes focus yet.

## Seams for the session work

- `Phase { Idle, Starting, Running, Ready, Failed }` and `set_phase`: the single place that
  changes the phase. It also maintains `unread` (`unread_after`: Ready behind a closed panel sets
  it, a new turn clears it, opening clears it); `is_busy` drives the header button's spinner
  and `has_unread` its highlight. Both are unused today, so they carry `#[allow(dead_code)]`;
  remove it with the first caller.
- `request_overview` and `can_request_overview`: the empty state's button calls the first and is
  disabled while the second is false, so no control is dead. The orchestration fills both:
  enabled when the lookup found the CLI and no turn runs.
- `apply_settings` is where a changed profile or command is noticed (`changed`); the session
  work ends the running process there. `start_lookup` can be called by anything that wants a
  fresh answer before launching.
- `Render for AgentPanel::render` lays out title row then a body; the message list replaces
  `render_empty` for a review with messages, the chat input goes under it, and `Phase::Failed`
  text is already shown in the empty state.
- Links and stops hang off the body of `ReviewView`, not off this entity: the panel only needs to
  raise events for them.

## Decisions

- The panel is a non-modal frame like the file tree, not a sheet like the comments, so a link
  can move the diff while the panel stays open.
- Open state is per window and never saved; only the width is a setting.
- The overview button stays visible but disabled until a session can start, rather than being
  absent: the empty state would otherwise invite something with no way to do it.
- Default key for `ToggleAgent` is ⌘/Ctrl-Shift-A (free, "assistant").
