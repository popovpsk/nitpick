use std::io::Cursor;
use std::time::{Duration, SystemTime};

use image::{ImageFormat, RgbaImage};
use rstest::rstest;
use tempfile::TempDir;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    RgbaImage::new(width, height)
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

fn dimensions(bytes: &[u8]) -> (u32, u32) {
    image::load_from_memory(bytes)
        .unwrap()
        .into_rgba8()
        .dimensions()
}

#[test]
fn any_picture_becomes_a_small_square_png() {
    let normalized = normalize(&png(120, 40)).unwrap();
    assert_eq!(image::guess_format(&normalized).unwrap(), ImageFormat::Png);
    assert_eq!(dimensions(&normalized), (SIZE, SIZE));
}

#[rstest]
#[case::tall(1, 100_000)]
#[case::wide(100_000, 1)]
fn a_thin_picture_is_cropped_before_it_is_scaled(#[case] width: u32, #[case] height: u32) {
    assert_eq!(
        dimensions(&normalize(&png(width, height)).unwrap()),
        (SIZE, SIZE)
    );
}

#[test]
fn oversized_or_broken_pictures_are_refused() {
    assert!(matches!(
        normalize_within(&png(20, 10), 100),
        Err(Error::TooManyPixels)
    ));
    assert!(normalize(b"<html>login</html>").is_err());
}

#[test]
fn the_cache_keys_by_url_and_ages_entries() {
    let dir = TempDir::new().unwrap();
    let cache = Cache::new(dir.path().join("avatars"));
    let url = "https://gitlab.example.com/uploads/a.png";
    assert!(cache.read(url, SystemTime::now()).is_none());
    cache.write(url, b"png").unwrap();
    let now = SystemTime::now();
    let fresh = cache.read(url, now).unwrap();
    assert_eq!((fresh.png.as_slice(), fresh.fresh), (&b"png"[..], true));
    let later = now + MAX_AGE + Duration::from_secs(60);
    assert!(!cache.read(url, later).unwrap().fresh);
    assert!(
        cache
            .read("https://gitlab.example.com/uploads/b.png", now)
            .is_none()
    );
    assert_ne!(cache.path(url), cache.path(&format!("{url}?v=2")));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn the_token_goes_only_to_the_forge_origin() {
    runtime().block_on(async {
        let forge = MockServer::start().await;
        let storage = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/uploads/a.png"))
            .and(header("private-token", "secret"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/blob/a.png", storage.uri())),
            )
            .mount(&forge)
            .await;
        Mock::given(method("GET"))
            .and(path("/blob/a.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image".to_vec()))
            .mount(&storage)
            .await;
        let downloader = Downloader::new().unwrap();
        let auth = Auth::gitlab(&forge.uri(), "secret").unwrap();
        let body = downloader
            .fetch(&format!("{}/uploads/a.png", forge.uri()), Some(&auth))
            .await
            .unwrap();
        assert_eq!(body, b"image");
        let leaked = storage
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request: &Request| request.headers.contains_key("private-token"));
        assert!(!leaked);
    });
}

#[test]
fn other_hosts_get_no_token() {
    runtime().block_on(async {
        let forge = MockServer::start().await;
        let elsewhere = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&elsewhere)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .mount(&elsewhere)
            .await;
        let auth = Auth::github(&forge.uri(), "token").unwrap();
        let body = Downloader::new()
            .unwrap()
            .fetch(&format!("{}/u/1", elsewhere.uri()), Some(&auth))
            .await
            .unwrap();
        assert_eq!(body, b"ok");
    });
}

#[rstest]
#[case::too_large(ResponseTemplate::new(200).set_body_bytes(vec![0u8; MAX_BYTES + 1]), "larger")]
#[case::not_found(ResponseTemplate::new(404), "404")]
fn failed_downloads_are_errors(#[case] response: ResponseTemplate, #[case] message: &str) {
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        let err = Downloader::new()
            .unwrap()
            .fetch(&format!("{}/a.png", server.uri()), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains(message), "{err}");
    });
}

#[rstest]
#[case::empty_token("https://gitlab.example.com", "  ")]
#[case::bad_url("not a url", "token")]
fn unusable_auth_is_none(#[case] base: &str, #[case] token: &str) {
    assert!(Auth::gitlab(base, token).is_none());
    assert!(Auth::github(base, token).is_none());
}

#[test]
fn auth_applies_to_its_own_origin_only() {
    let auth = Auth::gitlab("https://gitlab.example.com/", "t").unwrap();
    assert!(auth.applies_to("https://gitlab.example.com/uploads/a.png"));
    assert!(!auth.applies_to("http://gitlab.example.com/uploads/a.png"));
    assert!(!auth.applies_to("https://gitlab.example.com:8443/uploads/a.png"));
    assert!(!auth.applies_to("https://cdn.example.com/a.png"));
}
