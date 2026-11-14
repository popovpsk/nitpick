use crate::convert;
use crate::error::Error;
use crate::models::{Discussion, ForgeKind, NewPosition, Request, User};

#[derive(Debug, Clone)]
pub enum Client {
    GitLab(gitlab::Client),
    GitHub(github::Client),
}

impl Client {
    pub fn new(kind: ForgeKind, base_url: &str, token: &str) -> Result<Client, Error> {
        Ok(match kind {
            ForgeKind::GitLab => Client::GitLab(gitlab::Client::new(base_url, token)?),
            ForgeKind::GitHub => Client::GitHub(github::Client::new(base_url, token)?),
        })
    }

    pub fn kind(&self) -> ForgeKind {
        match self {
            Client::GitLab(_) => ForgeKind::GitLab,
            Client::GitHub(_) => ForgeKind::GitHub,
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Client::GitLab(client) => client.base_url(),
            Client::GitHub(client) => client.base_url(),
        }
    }

    pub async fn current_user(&self) -> Result<User, Error> {
        Ok(match self {
            Client::GitLab(client) => convert::gitlab_user(&client.current_user().await?),
            Client::GitHub(client) => convert::github_user(&client.current_user().await?),
        })
    }

    pub async fn open_requests(&self, project: &str) -> Result<Vec<Request>, Error> {
        Ok(match self {
            Client::GitLab(client) => client
                .merge_requests(&gitlab_project(project), &gitlab::MrQuery::default())
                .await?
                .iter()
                .map(convert::gitlab_request)
                .collect(),
            Client::GitHub(client) => client
                .pull_requests(&github::RepoRef::parse(project)?)
                .await?
                .iter()
                .map(convert::github_request)
                .collect(),
        })
    }

    pub async fn request(&self, project: &str, number: u64) -> Result<Request, Error> {
        Ok(match self {
            Client::GitLab(client) => convert::gitlab_request(
                &client
                    .merge_request(&gitlab_project(project), number)
                    .await?,
            ),
            Client::GitHub(client) => convert::github_request(
                &client
                    .pull_request(&github::RepoRef::parse(project)?, number)
                    .await?,
            ),
        })
    }

    pub async fn discussions(&self, project: &str, number: u64) -> Result<Vec<Discussion>, Error> {
        Ok(match self {
            Client::GitLab(client) => client
                .discussions(&gitlab_project(project), number)
                .await?
                .iter()
                .map(convert::gitlab_discussion)
                .collect(),
            Client::GitHub(client) => {
                let repo = github::RepoRef::parse(project)?;
                let threads = client.review_threads(&repo, number).await?;
                let comments = client.issue_comments(&repo, number).await?;
                convert::github_discussions(&threads, &comments)
            }
        })
    }

    pub async fn comment(
        &self,
        project: &str,
        number: u64,
        body: &str,
        position: Option<&NewPosition>,
    ) -> Result<(), Error> {
        match self {
            Client::GitLab(client) => {
                let position = position.map(convert::gitlab_position);
                client
                    .create_discussion(&gitlab_project(project), number, body, position.as_ref())
                    .await?;
            }
            Client::GitHub(client) => {
                let repo = github::RepoRef::parse(project)?;
                match position {
                    Some(position) => {
                        let comment = convert::github_comment(body, position)?;
                        client
                            .create_review_comment(&repo, number, &comment)
                            .await?;
                    }
                    None => {
                        client.create_issue_comment(&repo, number, body).await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn reply(
        &self,
        project: &str,
        number: u64,
        discussion_id: &str,
        body: &str,
    ) -> Result<(), Error> {
        match self {
            Client::GitLab(client) => {
                client
                    .reply(&gitlab_project(project), number, discussion_id, body)
                    .await?;
            }
            Client::GitHub(client) => {
                let repo = github::RepoRef::parse(project)?;
                let root = convert::github_reply_target(discussion_id)?;
                client.reply(&repo, number, root, body).await?;
            }
        }
        Ok(())
    }

    pub async fn set_resolved(
        &self,
        project: &str,
        number: u64,
        resolve_id: &str,
        resolved: bool,
    ) -> Result<(), Error> {
        match self {
            Client::GitLab(client) => {
                client
                    .set_resolved(&gitlab_project(project), number, resolve_id, resolved)
                    .await?;
            }
            Client::GitHub(client) => {
                client.set_thread_resolved(resolve_id, resolved).await?;
            }
        }
        Ok(())
    }
}

fn gitlab_project(project: &str) -> gitlab::ProjectRef {
    gitlab::ProjectRef::Path(project.to_string())
}
