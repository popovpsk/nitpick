use rstest::rstest;
use sha2::Digest as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{download_with, verify_digest};

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn hasher_of(bytes: &[u8]) -> sha2::Sha256 {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
}

fn payload() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn download_streams_the_body_to_disk_and_checks_the_digest() {
    let body = payload();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("asset.bin");
    let digest = format!("sha256:{}", sha256_hex(&body));
    let client = http_client::builder("sloptaster-test")
        .unwrap()
        .build()
        .unwrap();

    download_with(
        &client,
        &format!("{}/asset", server.uri()),
        &dest,
        Some(&digest),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

#[rstest]
#[case::matching(Some(format!("sha256:{}", sha256_hex(b"update"))), true)]
#[case::matching_upper_case(Some(format!("sha256:{}", sha256_hex(b"update").to_uppercase())), true)]
#[case::mismatch(Some(format!("sha256:{}", sha256_hex(b"other"))), false)]
#[case::missing(None, true)]
#[case::not_sha256(Some("sha512:abc".to_string()), true)]
fn digest_rule(#[case] digest: Option<String>, #[case] accepted: bool) {
    let result = verify_digest(hasher_of(b"update"), digest.as_deref());
    assert_eq!(result.is_ok(), accepted, "{result:?}");
}
