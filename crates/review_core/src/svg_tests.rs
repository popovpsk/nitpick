use std::io::Write as _;

use rstest::rstest;

use crate::svg::{SvgError, check_svg};

const LIMIT: u64 = 1024 * 1024;
const PIXELS: u64 = 50_000_000;
const PNG_10X10: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9";

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#
    )
}

fn check(data: Vec<u8>) -> Result<Vec<u8>, SvgError> {
    check_svg(data, LIMIT, PIXELS)
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn is_external(result: &Result<Vec<u8>, SvgError>) -> bool {
    matches!(result, Err(SvgError::ExternalReference { .. }))
}

#[rstest]
#[case::shapes_only(r#"<rect width="5" height="5"/>"#)]
#[case::embedded_png(
    r#"<image href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9" width="5" height="5"/>"#
)]
#[case::embedded_jpeg_xlink(r#"<image xlink:href="data:image/jpeg;base64,/9j/4A==" width="5"/>"#)]
#[case::unreadable_png(r#"<image href="data:image/png;base64,AAAA"/>"#)]
#[case::type_usvg_ignores(r#"<image href="data:image/bmp;base64,Qk0="/>"#)]
#[case::image_without_href(r#"<image width="5" height="5"/>"#)]
#[case::embedded_fe_image(
    r#"<filter id="f"><feImage href="data:image/png;base64,iVBORw0KGgo="/></filter>"#
)]
#[case::links_outside_images(
    r##"<a href="https://example.com"><use href="#dot"/></a><circle id="dot" r="2"/>"##
)]
fn keeps_self_contained_svgs(#[case] body: &str) {
    let text = svg(body);
    assert_eq!(check(text.clone().into_bytes()).unwrap(), text.into_bytes());
}

#[rstest]
#[case::absolute_path(r#"<image href="/dev/zero" width="5" height="5"/>"#)]
#[case::xlink_path(r#"<image xlink:href="/etc/hosts.png"/>"#)]
#[case::relative_path(r#"<image href="photo.png"/>"#)]
#[case::unc_path(r#"<image href="\\attacker.example\share\a.png"/>"#)]
#[case::file_url(r#"<image href="file:///etc/passwd"/>"#)]
#[case::web_url(r#"<image href="https://example.com/a.png"/>"#)]
#[case::nested_svg(r#"<image href="data:image/svg+xml;base64,PHN2Zy8+"/>"#)]
#[case::untyped_data(r#"<image href="data:,hello"/>"#)]
#[case::fragment_on_image(r##"<image href="#dot"/><circle id="dot" r="2"/>"##)]
#[case::fe_image_of_an_element(
    r##"<filter id="f"><feImage href="#dot"/></filter><circle id="dot" r="2"/>"##
)]
#[case::fe_image_of_a_title(
    r##"<title id="x">t</title><filter id="f"><feImage href="#x"/></filter><rect filter="url(#f)" width="5" height="5"/>"##
)]
#[case::fe_image_missing_element(r##"<filter id="f"><feImage href="#nowhere"/></filter>"##)]
#[case::fe_image_path(r#"<filter id="f"><feImage href="/dev/zero"/></filter>"#)]
fn refuses_references_outside_the_file(#[case] body: &str) {
    let result = check(svg(body).into_bytes());
    assert!(is_external(&result), "{result:?}");
}

#[rstest]
#[case::png(
    r#"<image href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAATiAAAE4gCAYAAADjcEY5"/>"#
)]
#[case::jpeg_typed_jpg(r#"<image href="data:image/jpg;base64,/9j/wAARCD6APoADASIAAhEBAxEB"/>"#)]
#[case::fe_image(
    r#"<filter id="f"><feImage href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAATiAAAE4gCAYAAADjcEY5"/></filter>"#
)]
fn refuses_embedded_pictures_over_the_pixel_limit(#[case] body: &str) {
    let result = check(svg(body).into_bytes());
    assert!(
        matches!(result, Err(SvgError::PictureTooLarge { .. })),
        "{result:?}"
    );
}

#[test]
fn the_pixel_limit_is_inclusive() {
    let text = svg(&format!(r#"<image href="{PNG_10X10}"/>"#));
    assert!(check_svg(text.clone().into_bytes(), LIMIT, 100).is_ok());
    let result = check_svg(text.into_bytes(), LIMIT, 99);
    assert!(
        matches!(
            result,
            Err(SvgError::PictureTooLarge {
                width: 10,
                height: 10
            })
        ),
        "{result:?}"
    );
}

#[test]
fn sees_an_href_hidden_in_an_entity() {
    let text = r#"<?xml version="1.0"?>
<!DOCTYPE svg [<!ENTITY target "/dev/zero">]>
<svg xmlns="http://www.w3.org/2000/svg"><image href="&target;"/></svg>"#;
    let result = check(text.as_bytes().to_vec());
    assert!(is_external(&result), "{result:?}");
}

#[test]
fn sees_an_image_inserted_by_an_entity() {
    let text = r#"<?xml version="1.0"?>
<!DOCTYPE svg [<!ENTITY picture "<image href='/dev/zero'/>">]>
<svg xmlns="http://www.w3.org/2000/svg">&picture;</svg>"#;
    let result = check(text.as_bytes().to_vec());
    assert!(is_external(&result), "{result:?}");
}

#[test]
fn unpacks_svgz() {
    let text = svg(r#"<rect width="5" height="5"/>"#);
    assert_eq!(check(gzip(text.as_bytes())).unwrap(), text.into_bytes());
}

#[test]
fn checks_inside_svgz() {
    let text = svg(r#"<image href="/dev/zero"/>"#);
    let result = check(gzip(text.as_bytes()));
    assert!(is_external(&result), "{result:?}");
}

#[test]
fn caps_the_unpacked_size() {
    let text = svg(&" ".repeat(4096));
    let result = check_svg(gzip(text.as_bytes()), 1024, PIXELS);
    assert!(
        matches!(result, Err(SvgError::TooLarge(1024))),
        "{result:?}"
    );
}

#[test]
fn refuses_svgz_inside_svgz() {
    let inner = gzip(svg("").as_bytes());
    let result = check(gzip(&inner));
    assert!(matches!(result, Err(SvgError::NotText)), "{result:?}");
}

#[test]
fn refuses_what_is_not_svg_text() {
    let binary = check(vec![0xff, 0xfe, 0x00]);
    assert!(matches!(binary, Err(SvgError::NotText)), "{binary:?}");
    let broken_gzip = check(vec![0x1f, 0x8b, 0x00]);
    assert!(
        matches!(broken_gzip, Err(SvgError::Gzip(_))),
        "{broken_gzip:?}"
    );
    let not_xml = check(b"<svg".to_vec());
    assert!(matches!(not_xml, Err(SvgError::Xml(_))), "{not_xml:?}");
}
