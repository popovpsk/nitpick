use crate::{Discussion, MergeRequest, Note};

pub(crate) const DIFF_DISCUSSION: &str = r#"{
  "id": "6a9c1750b37d513a43987b574953fceb50b03ce7",
  "individual_note": false,
  "notes": [
    {
      "id": 1128,
      "type": "DiffNote",
      "body": "Should this be `u64`?",
      "attachment": null,
      "author": {
        "id": 1,
        "name": "Venkatesh Thalluri",
        "username": "venkatesh.thalluri",
        "state": "active",
        "locked": false,
        "avatar_url": "https://www.gravatar.com/avatar/00afb8fb6ab07c3ee3e9c1f38777e2f4?s=80&d=identicon",
        "web_url": "https://gitlab.example.com/venkatesh.thalluri"
      },
      "created_at": "2018-03-04T09:17:22.520Z",
      "updated_at": "2018-03-04T09:17:22.520Z",
      "system": false,
      "noteable_id": 3,
      "noteable_type": "MergeRequest",
      "commit_id": null,
      "position": {
        "base_sha": "b5d6e7b1613fca24d250fa8e5bc7bcc3dd6002ef",
        "start_sha": "7c9c2ead8a320fb7ba0b4e234bd9529a2614e306",
        "head_sha": "4803c71e6b1833ca72b8b26ef2ecd5adc8a38031",
        "old_path": "src/lib.rs",
        "new_path": "src/lib.rs",
        "position_type": "text",
        "old_line": null,
        "new_line": 18,
        "line_range": {
          "start": {
            "line_code": "a0e9d0f6dcc5f4f0bd5a2ff2e1df6a6c3b7cbb0b_16_16",
            "type": null,
            "old_line": 16,
            "new_line": 16
          },
          "end": {
            "line_code": "a0e9d0f6dcc5f4f0bd5a2ff2e1df6a6c3b7cbb0b_17_18",
            "type": "new",
            "old_line": null,
            "new_line": 18
          }
        }
      },
      "resolved": false,
      "resolvable": true,
      "resolved_by": null,
      "resolved_at": null,
      "suggestions": [],
      "confidential": false,
      "internal": false,
      "imported": false,
      "imported_from": "none",
      "noteable_iid": 3,
      "commands_changes": {}
    },
    {
      "id": 1129,
      "type": "DiffNote",
      "body": "Yes, fixed.",
      "author": {
        "id": 2,
        "name": "Ada Reviewer",
        "username": "ada",
        "avatar_url": null
      },
      "created_at": "2018-03-04T10:00:00.000Z",
      "updated_at": null,
      "system": false,
      "noteable_type": "MergeRequest",
      "resolved": true,
      "resolvable": true,
      "resolved_by": {"id": 2, "username": "ada", "name": "Ada Reviewer"}
    }
  ]
}"#;

pub(crate) const GENERAL_DISCUSSION: &str = r#"{
  "id": "87805b7c09016a7058e91bdbe7b29d1f284a39e6",
  "individual_note": true,
  "notes": [
    {
      "id": 1126,
      "type": null,
      "body": "Looks good overall.",
      "attachment": null,
      "author": {"id": 3, "name": "Grace", "username": "grace", "state": "active", "avatar_url": null},
      "created_at": "2018-03-03T21:54:39.668Z",
      "updated_at": "2018-03-03T21:54:39.668Z",
      "system": false,
      "noteable_id": 3,
      "noteable_type": "MergeRequest",
      "resolvable": false,
      "noteable_iid": 3
    }
  ]
}"#;

pub(crate) const SYSTEM_DISCUSSION: &str = r#"{
  "id": "11aa",
  "individual_note": true,
  "notes": [
    {
      "id": 1000,
      "type": null,
      "body": "added 2 commits",
      "author": {"id": 3, "name": "Grace", "username": "grace"},
      "created_at": "2018-03-02T10:00:00.000Z",
      "system": true,
      "resolvable": false,
      "resolved": null
    }
  ]
}"#;

pub(crate) const MERGE_REQUEST: &str = r#"{
  "id": 155016530,
  "iid": 133,
  "project_id": 15513260,
  "title": "Manual job rules",
  "description": "Add rules for manual jobs",
  "state": "opened",
  "created_at": "2022-05-13T07:26:38.402Z",
  "updated_at": "2022-05-14T03:38:31.354Z",
  "merged_by": null,
  "merge_user": null,
  "merged_at": null,
  "closed_by": null,
  "closed_at": null,
  "target_branch": "main",
  "source_branch": "manual-job-rules",
  "user_notes_count": 4,
  "upvotes": 0,
  "downvotes": 0,
  "author": {
    "id": 4155490,
    "username": "marcel.amirault",
    "name": "Marcel Amirault",
    "state": "active",
    "avatar_url": "https://gitlab.com/uploads/-/system/user/avatar/4155490/avatar.png",
    "web_url": "https://gitlab.com/marcel.amirault"
  },
  "assignees": [],
  "reviewers": [],
  "source_project_id": 15513261,
  "target_project_id": 15513260,
  "labels": ["documentation"],
  "draft": true,
  "work_in_progress": true,
  "milestone": null,
  "merge_when_pipeline_succeeds": false,
  "merge_status": "can_be_merged",
  "detailed_merge_status": "not_approved",
  "sha": "e82eb4a098e32c796079ca3915e07487fc4db24c",
  "merge_commit_sha": null,
  "squash_commit_sha": null,
  "discussion_locked": null,
  "should_remove_source_branch": null,
  "force_remove_source_branch": true,
  "reference": "!133",
  "references": {"short": "!133", "relative": "!133", "full": "marcel.amirault/test-project!133"},
  "web_url": "https://gitlab.com/marcel.amirault/test-project/-/merge_requests/133",
  "time_stats": {"time_estimate": 0, "total_time_spent": 0},
  "squash": false,
  "task_completion_status": {"count": 0, "completed_count": 0},
  "has_conflicts": false,
  "blocking_discussions_resolved": true,
  "subscribed": true,
  "changes_count": "1",
  "diff_refs": {
    "base_sha": "1162f719d711319a2efb2a35566f3bfdadee8bab",
    "head_sha": "e82eb4a098e32c796079ca3915e07487fc4db24c",
    "start_sha": "1162f719d711319a2efb2a35566f3bfdadee8bab"
  },
  "merge_error": null,
  "first_contribution": false,
  "user": {"can_merge": true}
}"#;

pub(crate) fn json_list(items: &[&str]) -> String {
    format!("[{}]", items.join(","))
}

#[test]
fn diff_discussion_with_position_and_line_range() {
    let discussion: Discussion = serde_json::from_str(DIFF_DISCUSSION).unwrap();
    assert_eq!(discussion.id, "6a9c1750b37d513a43987b574953fceb50b03ce7");
    assert!(!discussion.individual_note);
    assert_eq!(discussion.notes.len(), 2);
    assert!(discussion.is_resolvable());
    assert!(!discussion.is_resolved());

    let position = discussion.position().unwrap();
    assert_eq!(position.position_type, "text");
    assert_eq!(position.new_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(position.old_line, None);
    assert_eq!(position.new_line, Some(18));
    assert_eq!(
        position.head_sha.as_deref(),
        Some("4803c71e6b1833ca72b8b26ef2ecd5adc8a38031")
    );
    let range = position.line_range.as_ref().unwrap();
    assert_eq!(range.start.line_type, None);
    assert_eq!(range.start.old_line, Some(16));
    assert_eq!(range.end.line_type.as_deref(), Some("new"));
    assert_eq!(range.end.new_line, Some(18));

    let first = &discussion.notes[0];
    assert_eq!(first.author.username, "venkatesh.thalluri");
    assert!(first.author.avatar_url.is_some());
    let reply = &discussion.notes[1];
    assert!(reply.resolved);
    assert_eq!(reply.updated_at, None);
    assert_eq!(reply.position, None);
}

#[test]
fn general_discussion_has_no_position() {
    let discussion: Discussion = serde_json::from_str(GENERAL_DISCUSSION).unwrap();
    assert!(discussion.individual_note);
    assert!(!discussion.is_resolvable());
    assert!(!discussion.is_resolved());
    assert_eq!(discussion.position(), None);
}

#[test]
fn system_notes_are_kept_and_null_resolved_reads_false() {
    let discussion: Discussion = serde_json::from_str(SYSTEM_DISCUSSION).unwrap();
    let note: &Note = &discussion.notes[0];
    assert!(note.system);
    assert!(!note.resolved);
    assert_eq!(note.updated_at, None);
}

#[test]
fn resolved_when_every_resolvable_note_is_resolved() {
    let mut discussion: Discussion = serde_json::from_str(DIFF_DISCUSSION).unwrap();
    discussion.notes[0].resolved = true;
    assert!(discussion.is_resolved());
}

#[test]
fn merge_request_reads_diff_refs_and_draft() {
    let mr: MergeRequest = serde_json::from_str(MERGE_REQUEST).unwrap();
    assert_eq!(mr.iid, 133);
    assert_eq!(mr.source_project_id, 15513261);
    assert_eq!(mr.target_project_id, 15513260);
    assert!(mr.draft);
    assert_eq!(mr.user_notes_count, 4);
    let refs = mr.diff_refs.unwrap();
    assert_eq!(refs.base_sha, "1162f719d711319a2efb2a35566f3bfdadee8bab");
    assert_eq!(refs.head_sha, "e82eb4a098e32c796079ca3915e07487fc4db24c");
}

#[test]
fn legacy_work_in_progress_means_draft() {
    let mut value: serde_json::Value = serde_json::from_str(MERGE_REQUEST).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("draft");
    object.remove("diff_refs");
    object.insert("description".into(), serde_json::Value::Null);
    let mr: MergeRequest = serde_json::from_value(value).unwrap();
    assert!(mr.draft);
    assert_eq!(mr.diff_refs, None);
    assert_eq!(mr.description, None);
}
