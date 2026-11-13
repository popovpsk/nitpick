use percent_encoding::percent_decode_str;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MrUrl {
    pub base_url: String,
    pub project_path: String,
    pub iid: u64,
}

pub fn parse_mr_url(url: &str) -> Option<MrUrl> {
    let url = parse_web_url(url)?;
    let all = segments(&url);
    let (project, iid) = split_mr_path(&all)?;
    Some(MrUrl {
        base_url: origin(&url),
        project_path: project.join("/"),
        iid,
    })
}

pub fn parse_mr_url_with_base(url: &str, base_url: &str) -> Option<MrUrl> {
    let parsed = parse_web_url(url)?;
    if let Some(base) = parse_web_url(base_url)
        && same_host(&parsed, &base)
    {
        let root = segments(&base);
        let all = segments(&parsed);
        if let Some((project, iid)) = all.strip_prefix(root.as_slice()).and_then(split_mr_path) {
            let mut base_url = origin(&base);
            for segment in &root {
                base_url.push('/');
                base_url.push_str(segment);
            }
            return Some(MrUrl {
                base_url,
                project_path: project.join("/"),
                iid,
            });
        }
    }
    parse_mr_url(url)
}

pub fn parse_mr_reference(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text.strip_prefix('!').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&iid| iid > 0)
}

fn parse_web_url(text: &str) -> Option<Url> {
    let url = Url::parse(text.trim()).ok()?;
    let web = matches!(url.scheme(), "http" | "https") && url.host_str().is_some();
    web.then_some(url)
}

fn segments(url: &Url) -> Vec<String> {
    url.path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .map(|segment| percent_decode_str(segment).decode_utf8_lossy().into_owned())
        .collect()
}

fn split_mr_path(segments: &[String]) -> Option<(&[String], u64)> {
    let separator = segments.iter().position(|segment| segment == "-")?;
    if separator == 0 || segments.get(separator + 1)? != "merge_requests" {
        return None;
    }
    let iid = parse_mr_reference(segments.get(separator + 2)?)?;
    Some((&segments[..separator], iid))
}

fn origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn same_host(a: &Url, b: &Url) -> bool {
    a.host_str() == b.host_str() && a.port() == b.port()
}
