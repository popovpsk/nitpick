use std::future::Future;
use std::io::Cursor;
use std::time::Duration;

use app_db::Db;

use image::{ImageFormat, ImageReader, imageops::FilterType};
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue, LOCATION};
use reqwest::redirect::Policy;
use url::{Origin, Url};

pub const SIZE: u32 = 64;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16_000_000;
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const MAX_HOPS: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = "nitpick";
const GITLAB_TOKEN: HeaderName = HeaderName::from_static("private-token");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid avatar URL: {0}")]
    InvalidUrl(String),
    #[error("avatar request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("avatar request failed with status {0}")]
    Status(u16),
    #[error("the avatar is larger than {MAX_BYTES} bytes")]
    TooLarge,
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("not an image: {0}")]
    Image(#[from] image::ImageError),
    #[error("the image is too large to decode")]
    TooManyPixels,
    #[error("could not set up HTTP: {0}")]
    Setup(String),
    #[error("avatar cache: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct Auth {
    origin: Origin,
    header: HeaderName,
    value: HeaderValue,
}

impl Auth {
    pub fn gitlab(base_url: &str, token: &str) -> Option<Auth> {
        Auth::new(base_url, GITLAB_TOKEN, token.trim().to_string())
    }

    pub fn github(base_url: &str, token: &str) -> Option<Auth> {
        Auth::new(base_url, AUTHORIZATION, format!("Bearer {}", token.trim()))
    }

    pub fn applies_to(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| url.origin() == self.origin)
    }

    fn new(base_url: &str, header: HeaderName, value: String) -> Option<Auth> {
        let origin = Url::parse(base_url.trim()).ok()?.origin();
        if !origin.is_tuple() || value.trim().is_empty() || value.trim() == "Bearer" {
            return None;
        }
        let mut value = HeaderValue::from_str(&value).ok()?;
        value.set_sensitive(true);
        Some(Auth {
            origin,
            header,
            value,
        })
    }
}

pub struct Downloader {
    following: reqwest::Client,
    manual: reqwest::Client,
}

impl Downloader {
    pub fn new() -> Result<Downloader, Error> {
        let builder = || {
            http_client::builder(USER_AGENT)
                .map(|b| b.timeout(TIMEOUT))
                .map_err(|e| Error::Setup(e.to_string()))
        };
        Ok(Downloader {
            following: builder()?.build()?,
            manual: builder()?.redirect(Policy::none()).build()?,
        })
    }

    pub async fn fetch(&self, url: &str, auth: Option<&Auth>) -> Result<Vec<u8>, Error> {
        let mut url = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        for _ in 0..MAX_HOPS {
            if !matches!(url.scheme(), "http" | "https") {
                return Err(Error::InvalidUrl(url.to_string()));
            }
            let authed = auth.filter(|auth| url.origin() == auth.origin);
            let response = match authed {
                Some(auth) => {
                    self.manual
                        .get(url.clone())
                        .header(auth.header.clone(), auth.value.clone())
                        .send()
                        .await?
                }
                None => self.following.get(url.clone()).send().await?,
            };
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(Error::Status(status.as_u16()))?;
                url = url
                    .join(location)
                    .map_err(|e| Error::InvalidUrl(e.to_string()))?;
                continue;
            }
            if !status.is_success() {
                return Err(Error::Status(status.as_u16()));
            }
            return read_capped(response).await;
        }
        Err(Error::TooManyRedirects)
    }
}

async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        return Err(Error::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    normalize_within(bytes, MAX_PIXELS)
}

fn normalize_within(bytes: &[u8], max_pixels: u64) -> Result<Vec<u8>, Error> {
    let reader = || ImageReader::new(Cursor::new(bytes)).with_guessed_format();
    let (width, height) = reader()?.into_dimensions()?;
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(Error::TooManyPixels);
    }
    let image = reader()?.decode()?;
    let side = width.min(height);
    let image = image
        .crop_imm((width - side) / 2, (height - side) / 2, side, side)
        .resize_exact(SIZE, SIZE, FilterType::Triangle);
    let mut png = Vec::new();
    image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
    Ok(png)
}

pub struct Cached {
    pub png: Vec<u8>,
    pub fresh: bool,
}

#[derive(Clone)]
pub struct Cache {
    db: Db,
}

impl Cache {
    pub fn new(db: Db) -> Cache {
        Cache { db }
    }

    pub fn read(
        &self,
        url: &str,
        now: i64,
    ) -> impl Future<Output = Option<Cached>> + 'static + use<> {
        let found = self.db.avatar(url);
        async move {
            let (png, fetched_at) = found
                .await
                .inspect_err(|err| log::debug!("avatar cache: {err}"))
                .ok()??;
            Some(Cached {
                png,
                fresh: now - fetched_at <= MAX_AGE.as_secs() as i64,
            })
        }
    }

    pub fn write(
        &self,
        url: &str,
        png: Vec<u8>,
        now: i64,
    ) -> impl Future<Output = ()> + 'static + use<> {
        let stored = self.db.put_avatar(url, png, now);
        async move {
            if let Err(err) = stored.await {
                log::debug!("avatar cache: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests;
