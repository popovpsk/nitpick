use percent_encoding::percent_decode_str;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrUrl {
    pub base_url: String,
    pub repo: String,
    pub number: u64,
}

pub fn parse_pr_url(url: &str) -> Option<PrUrl> {
    let url = parse_web_url(url)?;
    let all = segments(&url);
    let (repo, number) = split_pr_path(&all)?;
    Some(PrUrl {
        base_url: origin(&url),
        repo,
        number,
    })
}

pub fn parse_pr_url_with_base(url: &str, base_url: &str) -> Option<PrUrl> {
    let parsed = parse_web_url(url)?;
    if let Some(base) = parse_web_url(base_url)
        && same_host(&parsed, &base)
    {
        let root = segments(&base);
        let all = segments(&parsed);
        if let Some((repo, number)) = all.strip_prefix(root.as_slice()).and_then(split_pr_path) {
            let mut base_url = origin(&base);
            for segment in &root {
                base_url.push('/');
                base_url.push_str(segment);
            }
            return Some(PrUrl {
                base_url,
                repo,
                number,
            });
        }
    }
    parse_pr_url(url)
}

pub fn parse_pr_reference(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&number| number > 0)
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

fn split_pr_path(segments: &[String]) -> Option<(String, u64)> {
    match segments {
        [owner, name, pull, number, ..] if pull == "pull" => {
            let number = parse_pr_reference(number).filter(|_| !number.starts_with('#'))?;
            Some((format!("{owner}/{name}"), number))
        }
        _ => None,
    }
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
