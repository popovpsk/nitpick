use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, warn};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, LINK};
use reqwest::{Method, Response, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::models::{
    AddReviewData, GraphQlResponse, Ignored, IssueComment, PullRequest, ResolveData, Review,
    ReviewComment, ReviewThread, ThreadState, ThreadsData, User, group_threads,
};
use crate::request::{
    BodyOnly, GraphQlRequest, NewReviewComment, RepoRef, ReviewEvent, ReviewSubmit, ReviewVerdict,
    Side,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "nitpick";
const MEDIA_TYPE: &str = "application/vnd.github+json";
const API_VERSION_HEADER: HeaderName = HeaderName::from_static("x-github-api-version");
const API_VERSION: &str = "2022-11-28";
const PUBLIC_HOSTS: [&str; 3] = ["github.com", "www.github.com", "api.github.com"];
const PUBLIC_WEB: &str = "https://github.com";
const PUBLIC_API: &str = "https://api.github.com";
pub(crate) const PER_PAGE: u32 = 100;
pub(crate) const MAX_PULL_PAGES: usize = 3;
pub(crate) const MAX_COMMENT_PAGES: usize = 50;
pub(crate) const MAX_THREAD_PAGES: usize = 50;

const THREADS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!, $after: String) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { id isResolved viewerCanResolve viewerCanUnresolve comments(first: 1) { nodes { databaseId } } } } } } }";
const RESOLVE_MUTATION: &str = "mutation($id: ID!) { result: resolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }";
const ADD_REVIEW_MUTATION: &str = "mutation($pr: ID!, $commit: GitObjectID) { addPullRequestReview(input: {pullRequestId: $pr, commitOID: $commit}) { pullRequestReview { id } } }";
const ADD_THREAD_MUTATION: &str = "mutation($review: ID!, $path: String!, $line: Int!, $side: DiffSide!, $body: String!) { addPullRequestReviewThread(input: {pullRequestReviewId: $review, path: $path, line: $line, side: $side, body: $body}) { thread { id } } }";
const ADD_REPLY_MUTATION: &str = "mutation($review: ID!, $replyTo: ID!, $body: String!) { addPullRequestReviewComment(input: {pullRequestReviewId: $review, inReplyTo: $replyTo, body: $body}) { comment { id } } }";
const UPDATE_COMMENT_MUTATION: &str = "mutation($id: ID!, $body: String!) { updatePullRequestReviewComment(input: {pullRequestReviewCommentId: $id, body: $body}) { pullRequestReviewComment { id } } }";
const DELETE_COMMENT_MUTATION: &str = "mutation($id: ID!) { deletePullRequestReviewComment(input: {id: $id}) { pullRequestReview { id } } }";
const UNRESOLVE_MUTATION: &str = "mutation($id: ID!) { result: unresolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }";

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    api_root: String,
    graphql_url: String,
    api_origin: Url,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("api_root", &self.inner.api_root)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct ThreadsVariables<'a> {
    owner: &'a str,
    name: &'a str,
    number: u64,
    after: Option<&'a str>,
}

#[derive(Serialize)]
struct ThreadVariables<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct PullVariables<'a> {
    pr: &'a str,
    commit: Option<&'a str>,
}

#[derive(Serialize)]
struct ThreadDraftVariables<'a> {
    review: &'a str,
    path: &'a str,
    line: u32,
    side: Side,
    body: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyDraftVariables<'a> {
    review: &'a str,
    reply_to: &'a str,
    body: &'a str,
}

#[derive(Serialize)]
struct CommentBodyVariables<'a> {
    id: &'a str,
    body: &'a str,
}

impl Client {
    pub fn new(base_url: &str, token: &str) -> Result<Client, Error> {
        let web = parse_base_url(base_url)?;
        let public = web
            .host_str()
            .is_some_and(|host| PUBLIC_HOSTS.contains(&host));
        let (base_url, api_root, graphql_url) = if public {
            (
                PUBLIC_WEB.to_string(),
                PUBLIC_API.to_string(),
                format!("{PUBLIC_API}/graphql"),
            )
        } else if let Some(host) = web.host_str().filter(|host| host.ends_with(".ghe.com")) {
            let host = host.strip_prefix("api.").unwrap_or(host);
            (
                format!("https://{host}"),
                format!("https://api.{host}"),
                format!("https://api.{host}/graphql"),
            )
        } else {
            let base = web.as_str().trim_end_matches('/').to_string();
            let api = format!("{base}/api/v3");
            let graphql = format!("{base}/api/graphql");
            (base, api, graphql)
        };
        let api_origin = Url::parse(&api_root).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::InvalidToken);
        }
        let mut auth =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| Error::InvalidToken)?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        headers.insert(ACCEPT, HeaderValue::from_static(MEDIA_TYPE));
        headers.insert(API_VERSION_HEADER, HeaderValue::from_static(API_VERSION));
        let http = http_client::builder(USER_AGENT)?
            .default_headers(headers)
            .timeout(REQUEST_TIMEOUT)
            .redirect(http_client::same_host_redirects(&api_origin))
            .build()?;
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url,
                api_root,
                graphql_url,
                api_origin,
            }),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    pub fn api_root(&self) -> &str {
        &self.inner.api_root
    }

    pub async fn current_user(&self) -> Result<User, Error> {
        self.call(Method::GET, "/user", &[], None::<&()>).await
    }

    pub async fn pull_requests(&self, repo: &RepoRef) -> Result<Vec<PullRequest>, Error> {
        let path = format!("{}/pulls", repo.api_path());
        let query = [
            ("state", "open"),
            ("sort", "updated"),
            ("direction", "desc"),
        ];
        self.get_all(&path, &query, MAX_PULL_PAGES).await
    }

    pub async fn pull_request(&self, repo: &RepoRef, number: u64) -> Result<PullRequest, Error> {
        let path = pull_path(repo, number);
        self.call(Method::GET, &path, &[], None::<&()>).await
    }

    pub async fn review_comments(
        &self,
        repo: &RepoRef,
        number: u64,
    ) -> Result<Vec<ReviewComment>, Error> {
        let path = format!("{}/comments", pull_path(repo, number));
        self.get_all(&path, &[], MAX_COMMENT_PAGES).await
    }

    pub async fn issue_comments(
        &self,
        repo: &RepoRef,
        number: u64,
    ) -> Result<Vec<IssueComment>, Error> {
        let path = format!("{}/issues/{number}/comments", repo.api_path());
        self.get_all(&path, &[], MAX_COMMENT_PAGES).await
    }

    pub async fn thread_states(
        &self,
        repo: &RepoRef,
        number: u64,
    ) -> Result<Vec<ThreadState>, Error> {
        let mut states = Vec::new();
        let mut after: Option<String> = None;
        for page in 0.. {
            if page == MAX_THREAD_PAGES {
                warn!("{repo}#{number}: stopped after {MAX_THREAD_PAGES} pages of review threads");
                break;
            }
            let variables = ThreadsVariables {
                owner: &repo.owner,
                name: &repo.name,
                number,
                after: after.as_deref(),
            };
            let data: ThreadsData = self.graphql(THREADS_QUERY, variables).await?;
            let Some(connection) = data
                .repository
                .and_then(|r| r.pull_request)
                .map(|p| p.review_threads)
            else {
                return Err(Error::NotFound(format!("{repo}#{number} review threads")));
            };
            states.extend(
                connection
                    .nodes
                    .into_iter()
                    .flatten()
                    .filter_map(|node| node.into_state()),
            );
            match connection.page_info.end_cursor {
                Some(cursor) if connection.page_info.has_next_page => after = Some(cursor),
                _ => break,
            }
        }
        Ok(states)
    }

    pub async fn review_threads(
        &self,
        repo: &RepoRef,
        number: u64,
    ) -> Result<Vec<ReviewThread>, Error> {
        let comments = self.review_comments(repo, number).await?;
        let states = if comments.is_empty() {
            Vec::new()
        } else {
            self.thread_states(repo, number)
                .await
                .unwrap_or_else(|err| {
                    warn!("{repo}#{number}: review thread states unavailable: {err}");
                    Vec::new()
                })
        };
        Ok(group_threads(comments, &states))
    }

    pub async fn create_review_comment(
        &self,
        repo: &RepoRef,
        number: u64,
        comment: &NewReviewComment,
    ) -> Result<ReviewComment, Error> {
        let path = format!("{}/comments", pull_path(repo, number));
        self.call(Method::POST, &path, &[], Some(comment)).await
    }

    pub async fn reply(
        &self,
        repo: &RepoRef,
        number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, Error> {
        let path = format!("{}/comments/{comment_id}/replies", pull_path(repo, number));
        self.call(Method::POST, &path, &[], Some(&BodyOnly { body }))
            .await
    }

    pub async fn create_issue_comment(
        &self,
        repo: &RepoRef,
        number: u64,
        body: &str,
    ) -> Result<IssueComment, Error> {
        let path = format!("{}/issues/{number}/comments", repo.api_path());
        self.call(Method::POST, &path, &[], Some(&BodyOnly { body }))
            .await
    }

    pub async fn edit_review_comment(
        &self,
        repo: &RepoRef,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, Error> {
        let path = format!("{}/pulls/comments/{comment_id}", repo.api_path());
        self.call(Method::PATCH, &path, &[], Some(&BodyOnly { body }))
            .await
    }

    pub async fn delete_review_comment(
        &self,
        repo: &RepoRef,
        comment_id: u64,
    ) -> Result<(), Error> {
        let path = format!("{}/pulls/comments/{comment_id}", repo.api_path());
        self.send(Method::DELETE, &path, None::<&()>).await
    }

    pub async fn edit_issue_comment(
        &self,
        repo: &RepoRef,
        comment_id: u64,
        body: &str,
    ) -> Result<IssueComment, Error> {
        let path = format!("{}/issues/comments/{comment_id}", repo.api_path());
        self.call(Method::PATCH, &path, &[], Some(&BodyOnly { body }))
            .await
    }

    pub async fn delete_issue_comment(&self, repo: &RepoRef, comment_id: u64) -> Result<(), Error> {
        let path = format!("{}/issues/comments/{comment_id}", repo.api_path());
        self.send(Method::DELETE, &path, None::<&()>).await
    }

    pub async fn reviews(&self, repo: &RepoRef, number: u64) -> Result<Vec<Review>, Error> {
        let path = format!("{}/reviews", pull_path(repo, number));
        self.get_all(&path, &[], MAX_COMMENT_PAGES).await
    }

    pub async fn pending_review(
        &self,
        repo: &RepoRef,
        number: u64,
    ) -> Result<Option<Review>, Error> {
        Ok(self
            .reviews(repo, number)
            .await?
            .into_iter()
            .find(|review| review.state == "PENDING"))
    }

    pub async fn review_comments_of(
        &self,
        repo: &RepoRef,
        number: u64,
        review_id: u64,
    ) -> Result<Vec<ReviewComment>, Error> {
        let path = format!("{}/reviews/{review_id}/comments", pull_path(repo, number));
        self.get_all(&path, &[], MAX_COMMENT_PAGES).await
    }

    pub async fn review_comment(
        &self,
        repo: &RepoRef,
        comment_id: u64,
    ) -> Result<ReviewComment, Error> {
        let path = format!("{}/pulls/comments/{comment_id}", repo.api_path());
        self.call(Method::GET, &path, &[], None::<&()>).await
    }

    pub async fn start_review(
        &self,
        pull_node_id: &str,
        commit: Option<&str>,
    ) -> Result<String, Error> {
        let variables = PullVariables {
            pr: pull_node_id,
            commit,
        };
        let data: AddReviewData = self.graphql(ADD_REVIEW_MUTATION, variables).await?;
        data.result
            .and_then(|result| result.review)
            .map(|review| review.id)
            .ok_or_else(|| Error::Decode("/graphql: no review in the answer".to_string()))
    }

    pub async fn add_draft_thread(
        &self,
        review_node_id: &str,
        path: &str,
        line: u32,
        side: Side,
        body: &str,
    ) -> Result<(), Error> {
        let variables = ThreadDraftVariables {
            review: review_node_id,
            path,
            line,
            side,
            body,
        };
        let _: Ignored = self.graphql(ADD_THREAD_MUTATION, variables).await?;
        Ok(())
    }

    pub async fn add_draft_reply(
        &self,
        review_node_id: &str,
        reply_to_node_id: &str,
        body: &str,
    ) -> Result<(), Error> {
        let variables = ReplyDraftVariables {
            review: review_node_id,
            reply_to: reply_to_node_id,
            body,
        };
        let _: Ignored = self.graphql(ADD_REPLY_MUTATION, variables).await?;
        Ok(())
    }

    pub async fn update_draft(&self, comment_node_id: &str, body: &str) -> Result<(), Error> {
        let variables = CommentBodyVariables {
            id: comment_node_id,
            body,
        };
        let _: Ignored = self.graphql(UPDATE_COMMENT_MUTATION, variables).await?;
        Ok(())
    }

    pub async fn delete_draft(&self, comment_node_id: &str) -> Result<(), Error> {
        let _: Ignored = self
            .graphql(
                DELETE_COMMENT_MUTATION,
                ThreadVariables {
                    id: comment_node_id,
                },
            )
            .await?;
        Ok(())
    }

    pub async fn submit_review(
        &self,
        repo: &RepoRef,
        number: u64,
        pending: Option<u64>,
        verdict: ReviewVerdict,
        body: &str,
    ) -> Result<(), Error> {
        let submit = ReviewSubmit {
            event: verdict.event(),
            body,
        };
        let path = match pending {
            Some(id) => format!("{}/reviews/{id}/events", pull_path(repo, number)),
            None => format!("{}/reviews", pull_path(repo, number)),
        };
        self.send(Method::POST, &path, Some(&submit)).await
    }

    pub async fn discard_review(
        &self,
        repo: &RepoRef,
        number: u64,
        review_id: u64,
    ) -> Result<(), Error> {
        let path = format!("{}/reviews/{review_id}", pull_path(repo, number));
        self.send(Method::DELETE, &path, None::<&()>).await
    }

    pub async fn approve(&self, repo: &RepoRef, number: u64) -> Result<(), Error> {
        let path = format!("{}/reviews", pull_path(repo, number));
        self.send(Method::POST, &path, Some(&ReviewEvent { event: "APPROVE" }))
            .await
    }

    async fn send<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(), Error> {
        let url = self.url(path, &[])?;
        self.execute(method, url, path, body).await?;
        Ok(())
    }

    pub async fn set_thread_resolved(
        &self,
        thread_id: &str,
        resolved: bool,
    ) -> Result<bool, Error> {
        let query = if resolved {
            RESOLVE_MUTATION
        } else {
            UNRESOLVE_MUTATION
        };
        let data: ResolveData = self
            .graphql(query, ThreadVariables { id: thread_id })
            .await?;
        data.result
            .and_then(|r| r.thread)
            .map(|thread| thread.is_resolved)
            .ok_or_else(|| {
                Error::Decode(format!("/graphql: no thread in the answer for {thread_id}"))
            })
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url, Error> {
        let mut url = Url::parse(&format!("{}{path}", self.inner.api_root))
            .map_err(|e| Error::InvalidUrl(format!("{path}: {e}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    async fn get_all<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        max_pages: usize,
    ) -> Result<Vec<T>, Error> {
        let per_page = PER_PAGE.to_string();
        let mut params = query.to_vec();
        params.push(("per_page", per_page.as_str()));
        let mut next = Some(self.url(path, &params)?);
        let mut items = Vec::new();
        let mut pages = 0;
        while let Some(url) = next.take() {
            if pages == max_pages {
                warn!("{path}: stopped after {max_pages} pages");
                break;
            }
            pages += 1;
            let response = self.execute(Method::GET, url, path, None::<&()>).await?;
            next = self.next_page(&response, path);
            let page: Vec<T> = decode(response, path).await?;
            if page.is_empty() {
                break;
            }
            items.extend(page);
        }
        Ok(items)
    }

    fn next_page(&self, response: &Response, path: &str) -> Option<Url> {
        let link = response
            .headers()
            .get_all(LINK)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(next_link)?;
        match Url::parse(link) {
            Ok(url) if same_origin(&url, &self.inner.api_origin) => Some(url),
            _ => {
                warn!("{path}: not following a next-page link outside the GitHub API");
                None
            }
        }
    }

    async fn graphql<T, V>(&self, query: &str, variables: V) -> Result<T, Error>
    where
        T: DeserializeOwned,
        V: Serialize,
    {
        const PATH: &str = "/graphql";
        let url = Url::parse(&self.inner.graphql_url)
            .map_err(|e| Error::InvalidUrl(format!("{PATH}: {e}")))?;
        let body = GraphQlRequest { query, variables };
        let response = self.execute(Method::POST, url, PATH, Some(&body)).await?;
        let answer: GraphQlResponse<T> = decode(response, PATH).await?;
        if !answer.errors.is_empty() {
            let messages: Vec<String> = answer.errors.into_iter().map(|e| e.message).collect();
            return Err(Error::GraphQl(messages.join("; ")));
        }
        answer
            .data
            .ok_or_else(|| Error::Decode(format!("{PATH}: no data")))
    }

    async fn call<T, B>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&B>,
    ) -> Result<T, Error>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let url = self.url(path, query)?;
        let response = self.execute(method, url, path, body).await?;
        decode(response, path).await
    }

    async fn execute<B>(
        &self,
        method: Method,
        url: Url,
        path: &str,
        body: Option<&B>,
    ) -> Result<Response, Error>
    where
        B: Serialize + ?Sized,
    {
        debug!("GitHub {method} {path}");
        let mut request = self.inner.http.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(Error::from_status(status.as_u16(), path, &body))
    }
}

async fn decode<T: DeserializeOwned>(response: Response, path: &str) -> Result<T, Error> {
    let bytes = response.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|e| Error::Decode(format!("{path}: {e}")))
}

fn pull_path(repo: &RepoRef, number: u64) -> String {
    format!("{}/pulls/{number}", repo.api_path())
}

fn parse_base_url(raw: &str) -> Result<Url, Error> {
    let raw = raw.trim();
    let invalid = |reason: &str| Error::InvalidUrl(format!("{raw}: {reason}"));
    let with_scheme;
    let parsed = if raw.contains("://") {
        raw
    } else {
        with_scheme = format!("https://{raw}");
        &with_scheme
    };
    let mut url = Url::parse(parsed).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("only http and https are supported"));
    }
    if url.host_str().is_none() {
        return Err(invalid("no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("credentials in the URL are not supported"));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/api/v3").unwrap_or(path).to_string();
    url.set_path(&path);
    Ok(url)
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn next_link(header: &str) -> Option<&str> {
    header.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let target = pieces.next()?.trim();
        let target = target.strip_prefix('<')?.strip_suffix('>')?;
        pieces
            .any(|param| {
                let param = param.trim();
                param == "rel=\"next\"" || param == "rel=next"
            })
            .then_some(target)
    })
}
