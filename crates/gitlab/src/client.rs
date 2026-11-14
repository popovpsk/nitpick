use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, warn};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LINK};
use reqwest::{Method, Response, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::models::{Discussion, MergeRequest, Note, Project, User};
use crate::request::{
    MrQuery, NewDiscussionBody, NewPosition, NoteBody, ProjectRef, WirePosition, encode_segment,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "lgtm-ish";
const TOKEN_HEADER: HeaderName = HeaderName::from_static("private-token");
const NEXT_PAGE_HEADER: &str = "x-next-page";
pub(crate) const DISCUSSIONS_PER_PAGE: u32 = 100;
pub(crate) const MAX_PAGES: usize = 50;

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    api_root: String,
    origin: Url,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(base_url: &str, token: &str) -> Result<Client, Error> {
        let origin = parse_base_url(base_url)?;
        let base_url = origin.as_str().trim_end_matches('/').to_string();
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::InvalidToken);
        }
        let mut token = HeaderValue::from_str(token).map_err(|_| Error::InvalidToken)?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(TOKEN_HEADER, token);
        let http = http_client::builder(USER_AGENT)?
            .default_headers(headers)
            .timeout(REQUEST_TIMEOUT)
            .redirect(http_client::same_host_redirects(&origin))
            .build()?;
        let api_root = format!("{base_url}/api/v4");
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url,
                api_root,
                origin,
            }),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    pub async fn current_user(&self) -> Result<User, Error> {
        self.call(Method::GET, "/user", &[], None::<&()>).await
    }

    pub async fn project(&self, project: &ProjectRef) -> Result<Project, Error> {
        let path = format!("/projects/{}", project.api_segment());
        self.call(Method::GET, &path, &[], None::<&()>).await
    }

    pub async fn merge_requests(
        &self,
        project: &ProjectRef,
        query: &MrQuery,
    ) -> Result<Vec<MergeRequest>, Error> {
        let path = format!("/projects/{}/merge_requests", project.api_segment());
        let per_page = query.per_page.clamp(1, 100).to_string();
        let mut params = vec![
            ("state", query.state.as_str()),
            ("order_by", "updated_at"),
            ("sort", "desc"),
            ("per_page", per_page.as_str()),
        ];
        let optional = [
            ("search", &query.search),
            ("source_branch", &query.source_branch),
            ("target_branch", &query.target_branch),
        ];
        for (key, value) in optional {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                params.push((key, value));
            }
        }
        self.call(Method::GET, &path, &params, None::<&()>).await
    }

    pub async fn merge_request(
        &self,
        project: &ProjectRef,
        iid: u64,
    ) -> Result<MergeRequest, Error> {
        let path = mr_path(project, iid);
        self.call(Method::GET, &path, &[], None::<&()>).await
    }

    pub async fn discussions(
        &self,
        project: &ProjectRef,
        iid: u64,
    ) -> Result<Vec<Discussion>, Error> {
        let path = format!("{}/discussions", mr_path(project, iid));
        let mut next = Some(self.page_url(&path, 1)?);
        let mut discussions = Vec::new();
        let mut pages = 0;
        while let Some(url) = next.take() {
            if pages == MAX_PAGES {
                warn!("{path}: stopped after {MAX_PAGES} pages of discussions");
                break;
            }
            pages += 1;
            let response = self.execute(Method::GET, url, &path, None::<&()>).await?;
            next = self.next_page(&response, &path)?;
            let page: Vec<Discussion> = decode(response, &path).await?;
            if page.is_empty() {
                break;
            }
            discussions.extend(page);
        }
        Ok(discussions)
    }

    pub async fn create_discussion(
        &self,
        project: &ProjectRef,
        iid: u64,
        body: &str,
        position: Option<&NewPosition>,
    ) -> Result<Discussion, Error> {
        let path = format!("{}/discussions", mr_path(project, iid));
        let payload = NewDiscussionBody {
            body,
            position: position.map(WirePosition::from),
        };
        self.call(Method::POST, &path, &[], Some(&payload)).await
    }

    pub async fn reply(
        &self,
        project: &ProjectRef,
        iid: u64,
        discussion_id: &str,
        body: &str,
    ) -> Result<Note, Error> {
        let path = format!("{}/notes", discussion_path(project, iid, discussion_id));
        self.call(Method::POST, &path, &[], Some(&NoteBody { body }))
            .await
    }

    pub async fn set_resolved(
        &self,
        project: &ProjectRef,
        iid: u64,
        discussion_id: &str,
        resolved: bool,
    ) -> Result<Discussion, Error> {
        let path = discussion_path(project, iid, discussion_id);
        let resolved = if resolved { "true" } else { "false" };
        self.call(Method::PUT, &path, &[("resolved", resolved)], None::<&()>)
            .await
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url, Error> {
        let mut url = Url::parse(&format!("{}{path}", self.inner.api_root))
            .map_err(|e| Error::InvalidUrl(format!("{path}: {e}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    fn page_url(&self, path: &str, page: u32) -> Result<Url, Error> {
        let per_page = DISCUSSIONS_PER_PAGE.to_string();
        let page = page.to_string();
        self.url(path, &[("per_page", &per_page), ("page", &page)])
    }

    fn next_page(&self, response: &Response, path: &str) -> Result<Option<Url>, Error> {
        let headers = response.headers();
        let next_page = headers
            .get(NEXT_PAGE_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(next_page) = next_page {
            return match next_page.parse::<u32>() {
                Ok(page) => self.page_url(path, page).map(Some),
                Err(_) => {
                    warn!("{path}: ignoring malformed {NEXT_PAGE_HEADER} header");
                    Ok(None)
                }
            };
        }
        let link = headers
            .get_all(LINK)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(next_link);
        let Some(link) = link else {
            return Ok(None);
        };
        match Url::parse(link) {
            Ok(url) if same_origin(&url, &self.inner.origin) => Ok(Some(url)),
            _ => {
                warn!("{path}: not following a next-page link outside the GitLab instance");
                Ok(None)
            }
        }
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
        debug!("GitLab {method} {path}");
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

fn mr_path(project: &ProjectRef, iid: u64) -> String {
    format!("/projects/{}/merge_requests/{iid}", project.api_segment())
}

fn discussion_path(project: &ProjectRef, iid: u64, discussion_id: &str) -> String {
    format!(
        "{}/discussions/{}",
        mr_path(project, iid),
        encode_segment(discussion_id)
    )
}

fn parse_base_url(raw: &str) -> Result<Url, Error> {
    let raw = raw.trim();
    let invalid = |reason: &str| Error::InvalidUrl(format!("{raw}: {reason}"));
    let mut url = Url::parse(raw).map_err(|e| invalid(&e.to_string()))?;
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
    let path = path.strip_suffix("/api/v4").unwrap_or(path).to_string();
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
