#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteLocation {
    pub host: String,
    pub path: String,
    pub https_base: String,
}

pub fn parse_remote_url(url: &str) -> Option<RemoteLocation> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        let (authority, path) = rest.split_once('/')?;
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (host, port) = split_port(host_port);
        if host.is_empty() {
            return None;
        }
        let path = project_path(path)?;
        match scheme.as_str() {
            "https" | "http" => {
                let host = match port {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                Some(RemoteLocation {
                    https_base: format!("{scheme}://{host}"),
                    host,
                    path,
                })
            }
            "ssh" | "git+ssh" | "ssh+git" | "git" => Some(RemoteLocation {
                host: host.to_string(),
                path,
                https_base: format!("https://{host}"),
            }),
            _ => None,
        }
    } else {
        let (before, path) = url.split_once(':')?;
        if before.contains('/') || before.contains('\\') {
            return None;
        }
        let host = before.rsplit_once('@').map_or(before, |(_, host)| host);
        if host.is_empty() || host.len() == 1 {
            return None;
        }
        let path = project_path(path)?;
        Some(RemoteLocation {
            host: host.to_string(),
            path,
            https_base: format!("https://{host}"),
        })
    }
}

fn split_port(host_port: &str) -> (&str, Option<&str>) {
    let (host, port) = if host_port.starts_with('[') {
        match host_port.find(']') {
            Some(end) => {
                let (host, rest) = host_port.split_at(end + 1);
                (host, rest.strip_prefix(':'))
            }
            None => (host_port, None),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    let port = port.filter(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()));
    (host, port)
}

fn project_path(path: &str) -> Option<String> {
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    (!path.is_empty()).then(|| path.to_string())
}
