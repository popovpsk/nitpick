use chrono::FixedOffset;
use rstest::rstest;

use super::comments::opens_in_browser;
use super::image_preview::first_frame_png;
use super::{nearest_unviewed, note_time};

#[rstest]
#[case::gitlab_utc("2024-05-01T10:20:30.123Z", "2024-05-01 13:20")]
#[case::github_utc("2024-05-01T23:50:00Z", "2024-05-02 02:50")]
#[case::server_offset("2024-05-01T10:20:30+02:00", "2024-05-01 11:20")]
#[case::unparsable("2024-05-01T10:20", "2024-05-01 10:20")]
fn comment_times_show_in_the_local_zone(#[case] created: &str, #[case] shown: &str) {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    assert_eq!(note_time(created, &moscow), shown);
}

#[rstest]
#[case::below_first(&[false, true, false, false, false], Some(4))]
#[case::nearest_above(&[false, true, false, true, true], Some(2))]
#[case::none_left(&[true, true, true, true, true], None)]
fn advancing_prefers_below_then_the_nearest_above(
    #[case] viewed: &[bool],
    #[case] expected: Option<usize>,
) {
    let order = [0, 1, 2, 3, 4];
    assert_eq!(nearest_unviewed(&order[4..], &order[..3], viewed), expected);
}

#[rstest]
#[case::https("https://gitlab.example.com/a", true)]
#[case::http_upper("HTTP://example.com", true)]
#[case::mail("mailto:a@example.com", true)]
#[case::file("file:///Applications/Calculator.app", false)]
#[case::unc("file://\\\\host\\share\\x.exe", false)]
#[case::custom("vscode://file/etc/passwd", false)]
#[case::relative("/uploads/a.png", false)]
#[case::javascript("javascript:alert(1)", false)]
fn only_web_links_open(#[case] url: &str, #[case] opens: bool) {
    assert_eq!(opens_in_browser(url), opens);
}

#[test]
fn an_animated_gif_previews_as_its_first_frame() {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};
    let mut gif = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut gif);
        for shade in [10u8, 200] {
            let frame = RgbaImage::from_pixel(4, 3, Rgba([shade, shade, shade, 255]));
            encoder
                .encode_frame(Frame::from_parts(
                    frame,
                    0,
                    0,
                    Delay::from_numer_denom_ms(10, 1),
                ))
                .unwrap();
        }
    }
    let png = first_frame_png(&gif).unwrap();
    assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
    let frame = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(frame.dimensions(), (4, 3));
    assert_eq!(frame.get_pixel(0, 0)[0], 10);
}
