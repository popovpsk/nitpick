use crate::models::ForgeKind;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestUrl {
    pub kind: ForgeKind,
    pub project: String,
    pub number: u64,
}

pub fn parse_request_url(text: &str, gitlab_base: &str, github_base: &str) -> Option<RequestUrl> {
    if let Some(url) = gitlab::parse_mr_url_with_base(text, gitlab_base) {
        return Some(RequestUrl {
            kind: ForgeKind::GitLab,
            project: url.project_path,
            number: url.iid,
        });
    }
    github::parse_pr_url_with_base(text, github_base).map(|url| RequestUrl {
        kind: ForgeKind::GitHub,
        project: url.repo,
        number: url.number,
    })
}

pub fn parse_reference(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.starts_with('#') {
        github::parse_pr_reference(text)
    } else {
        gitlab::parse_mr_reference(text)
    }
}
