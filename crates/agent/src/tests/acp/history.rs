use serde_json::{Value, json};

use crate::acp::history::Replay;
use crate::prompt::instructions;
use crate::{Entry, EntryRole};

fn chunk(kind: &str, message: Option<&str>, text: &str) -> Value {
    let mut update = json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}});
    if let Some(message) = message {
        update["messageId"] = json!(message);
    }
    json!({"sessionId": "s", "update": update})
}

fn entry(role: EntryRole, text: &str) -> Entry {
    Entry {
        role,
        text: text.into(),
    }
}

fn replayed(updates: &[Value]) -> Vec<Entry> {
    replayed_with("", updates)
}

fn replayed_with(current: &str, updates: &[Value]) -> Vec<Entry> {
    let mut replay = Replay::new("s", current);
    for update in updates {
        replay.feed(update);
    }
    replay.finish()
}

#[test]
fn the_instructions_block_of_a_first_prompt_is_dropped() {
    let rules = instructions("", true, "");
    assert_eq!(
        replayed_with(
            &rules,
            &[
                chunk("user_message_chunk", Some("u1"), &rules),
                chunk("user_message_chunk", Some("u1"), "hello"),
                chunk("agent_message_chunk", Some("a1"), "hi"),
            ]
        ),
        vec![
            entry(EntryRole::User, "hello"),
            entry(EntryRole::Assistant, "hi")
        ]
    );
}

#[test]
fn instructions_joined_to_the_first_prompt_are_cut_off() {
    let rules = instructions("", true, "");
    let joined = format!("{rules}\n# Review context\n\nbrief\nWhy?");
    let expected = vec![entry(EntryRole::User, "# Review context\n\nbrief\nWhy?")];
    assert_eq!(
        replayed_with(&rules, &[chunk("user_message_chunk", Some("u1"), &joined)]),
        expected
    );
    let changed = instructions("Russian", false, "Short.\n\nMore.");
    let joined = format!("{changed}\n# Review context\n\nbrief\nWhy?");
    assert_eq!(
        replayed_with(&rules, &[chunk("user_message_chunk", Some("u1"), &joined)]),
        vec![entry(EntryRole::User, "# Review context\n\nbrief\nWhy?")]
    );
}

#[test]
fn chunks_of_one_message_are_joined_and_messages_of_one_turn_are_paragraphs() {
    assert_eq!(
        replayed(&[
            chunk("user_message_chunk", Some("u1"), "one "),
            chunk("user_message_chunk", Some("u1"), "question"),
            chunk("agent_message_chunk", Some("a1"), "Look"),
            chunk("agent_message_chunk", Some("a1"), "ing."),
            json!({"sessionId": "s", "update": {"sessionUpdate": "tool_call", "toolCallId": "t"}}),
            chunk("agent_message_chunk", Some("a2"), "Done."),
        ]),
        vec![
            entry(EntryRole::User, "one question"),
            entry(EntryRole::Assistant, "Looking.\n\nDone.")
        ]
    );
}

#[test]
fn two_prompts_without_an_answer_between_them_stay_apart() {
    assert_eq!(
        replayed(&[
            chunk("user_message_chunk", Some("u1"), "first"),
            chunk("user_message_chunk", Some("u2"), "second"),
        ]),
        vec![
            entry(EntryRole::User, "first"),
            entry(EntryRole::User, "second")
        ]
    );
}

#[test]
fn a_changed_text_without_a_user_part_is_cut_at_its_end() {
    let rules = instructions("", true, "");
    let changed = instructions("Russian", true, "");
    let joined = format!("{changed}\nhello");
    assert_eq!(
        replayed_with(&rules, &[chunk("user_message_chunk", None, &joined)]),
        vec![entry(EntryRole::User, "hello")]
    );
}

#[test]
fn other_sessions_thoughts_and_non_text_are_ignored() {
    let mut other = chunk("user_message_chunk", None, "elsewhere");
    other["sessionId"] = json!("t");
    assert_eq!(
        replayed(&[
            other,
            chunk("agent_thought_chunk", None, "thinking"),
            json!({"sessionId": "s", "update": {"sessionUpdate": "user_message_chunk",
                "content": {"type": "image", "data": ""}}}),
            chunk("agent_message_chunk", None, "answer"),
        ]),
        vec![entry(EntryRole::Assistant, "answer")]
    );
}
