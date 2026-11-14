use crate::error::Error;
use crate::models::{
    DiffRefs, Discussion, ForgeKind, NewPosition, Note, Position, Request, RequestState, User,
};

const ISSUE_PREFIX: &str = "issue-";

pub(crate) fn gitlab_user(user: &gitlab::User) -> User {
    User {
        username: user.username.clone(),
        name: if user.name.trim().is_empty() {
            user.username.clone()
        } else {
            user.name.clone()
        },
    }
}

pub(crate) fn gitlab_request(mr: &gitlab::MergeRequest) -> Request {
    Request {
        kind: ForgeKind::GitLab,
        number: mr.iid,
        title: mr.title.clone(),
        author: gitlab_user(&mr.author),
        source_branch: mr.source_branch.clone(),
        target_branch: mr.target_branch.clone(),
        web_url: mr.web_url.clone(),
        state: match mr.state.as_str() {
            "merged" => RequestState::Merged,
            "closed" => RequestState::Closed,
            _ => RequestState::Open,
        },
        draft: mr.draft,
        diff_refs: mr.diff_refs.as_ref().map(|refs| DiffRefs {
            base_sha: Some(refs.base_sha.clone()),
            start_sha: refs.start_sha.clone(),
            head_sha: refs.head_sha.clone(),
        }),
    }
}

pub(crate) fn gitlab_discussion(discussion: &gitlab::Discussion) -> Discussion {
    let position = discussion.position().map(|p| Position {
        base_sha: p.base_sha.clone(),
        start_sha: p.start_sha.clone(),
        head_sha: p.head_sha.clone(),
        old_path: p.old_path.clone(),
        new_path: p.new_path.clone(),
        old_line: p.old_line,
        new_line: p.new_line,
    });
    Discussion {
        id: discussion.id.clone(),
        notes: discussion
            .notes
            .iter()
            .map(|note| Note {
                id: note.id,
                body: note.body.clone(),
                author: gitlab_user(&note.author),
                created_at: note.created_at.clone(),
                system: note.system,
            })
            .collect(),
        position,
        can_reply: true,
        resolved: discussion.is_resolved(),
        resolve_id: discussion.is_resolvable().then(|| discussion.id.clone()),
    }
}

pub(crate) fn gitlab_position(position: &NewPosition) -> gitlab::NewPosition {
    gitlab::NewPosition {
        base_sha: position.base_sha.clone(),
        start_sha: position.start_sha.clone(),
        head_sha: position.head_sha.clone(),
        old_path: position.old_path.clone(),
        new_path: position.new_path.clone(),
        old_line: position.old_line,
        new_line: position.new_line,
    }
}

pub(crate) fn github_user(user: &github::User) -> User {
    User {
        username: user.login.clone(),
        name: user.display_name().to_string(),
    }
}

pub(crate) fn github_request(pr: &github::PullRequest) -> Request {
    let state = if pr.is_merged() {
        RequestState::Merged
    } else if pr.state == "closed" {
        RequestState::Closed
    } else {
        RequestState::Open
    };
    let diff_refs = (!pr.head.sha.is_empty() && !pr.base.sha.is_empty()).then(|| DiffRefs {
        base_sha: None,
        start_sha: pr.base.sha.clone(),
        head_sha: pr.head.sha.clone(),
    });
    Request {
        kind: ForgeKind::GitHub,
        number: pr.number,
        title: pr.title.clone(),
        author: github_user(&pr.user),
        source_branch: pr.head.name.clone(),
        target_branch: pr.base.name.clone(),
        web_url: pr.html_url.clone(),
        state,
        draft: pr.draft,
        diff_refs,
    }
}

pub(crate) fn github_thread(thread: &github::ReviewThread) -> Discussion {
    let root = thread.root();
    let (sha, line) = match root.line {
        Some(line) => (&root.commit_id, Some(line)),
        None => (&root.original_commit_id, root.original_line),
    };
    let (old_line, new_line) = match root.side.unwrap_or_default() {
        github::Side::Left => (line, None),
        github::Side::Right => (None, line),
    };
    let path = (!root.path.is_empty()).then(|| root.path.clone());
    let position = Position {
        base_sha: None,
        start_sha: None,
        head_sha: (!sha.is_empty()).then(|| sha.clone()),
        old_path: path.clone(),
        new_path: path,
        old_line,
        new_line,
    };
    let resolved = thread.state.as_ref().is_some_and(|s| s.resolved);
    let resolve_id = thread
        .state
        .as_ref()
        .filter(|s| {
            if s.resolved {
                s.can_unresolve
            } else {
                s.can_resolve
            }
        })
        .map(|s| s.id.clone());
    Discussion {
        id: root.id.to_string(),
        notes: thread
            .comments
            .iter()
            .map(|c| Note {
                id: c.id,
                body: c.body.clone(),
                author: github_user(&c.user),
                created_at: c.created_at.clone(),
                system: false,
            })
            .collect(),
        position: Some(position),
        can_reply: root.in_reply_to_id.is_none(),
        resolved,
        resolve_id,
    }
}

pub(crate) fn github_issue_comment(comment: &github::IssueComment) -> Discussion {
    Discussion {
        id: format!("{ISSUE_PREFIX}{}", comment.id),
        notes: vec![Note {
            id: comment.id,
            body: comment.body.clone(),
            author: github_user(&comment.user),
            created_at: comment.created_at.clone(),
            system: false,
        }],
        position: None,
        can_reply: false,
        resolved: false,
        resolve_id: None,
    }
}

pub(crate) fn github_discussions(
    threads: &[github::ReviewThread],
    comments: &[github::IssueComment],
) -> Vec<Discussion> {
    let mut all: Vec<Discussion> = threads
        .iter()
        .map(github_thread)
        .chain(comments.iter().map(github_issue_comment))
        .collect();
    all.sort_by(|a, b| first_time(a).cmp(first_time(b)));
    all
}

fn first_time(discussion: &Discussion) -> &str {
    discussion
        .notes
        .first()
        .map_or("", |note| note.created_at.as_str())
}

pub(crate) fn github_comment(
    body: &str,
    position: &NewPosition,
) -> Result<github::NewReviewComment, Error> {
    let (line, side) = match (position.new_line, position.old_line) {
        (Some(line), _) => (line, github::Side::Right),
        (None, Some(line)) => (line, github::Side::Left),
        (None, None) => return Err(Error::InvalidPosition),
    };
    Ok(github::NewReviewComment {
        body: body.to_string(),
        commit_id: position.head_sha.clone(),
        path: position.new_path.clone(),
        line,
        side,
    })
}

pub(crate) fn github_reply_target(discussion_id: &str) -> Result<u64, Error> {
    discussion_id
        .parse()
        .map_err(|_| Error::CannotReply(discussion_id.to_string()))
}
