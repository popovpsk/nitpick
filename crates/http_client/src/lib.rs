use std::sync::Arc;
use std::time::Duration;

use log::warn;
use reqwest::redirect::Policy;
use rustls_platform_verifier::BuilderVerifierExt;
use url::Url;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_REDIRECTS: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not set up TLS: {0}")]
    Tls(String),
}

pub fn builder(user_agent: &str) -> Result<reqwest::ClientBuilder, Error> {
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config()?)
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .redirect(Policy::limited(MAX_REDIRECTS)))
}

pub fn same_host_redirects(origin: &Url) -> Policy {
    let origin = origin.clone();
    Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if redirect_allowed(&origin, attempt.url()) {
            attempt.follow()
        } else {
            warn!("not following a redirect to another host");
            attempt.stop()
        }
    })
}

pub fn redirect_allowed(origin: &Url, target: &Url) -> bool {
    let upgrade_or_same = target.scheme() == origin.scheme() || target.scheme() == "https";
    target.host_str() == origin.host_str() && upgrade_or_same
}

fn tls_config() -> Result<rustls::ClientConfig, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_platform_verifier()
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

#[cfg(test)]
mod tests;
