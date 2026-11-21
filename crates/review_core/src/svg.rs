use std::io::Read as _;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const SHOWN_HREF_CHARS: usize = 80;

#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error("the unpacked SVG is larger than {0} bytes")]
    TooLarge(u64),
    #[error("not a valid svgz: {0}")]
    Gzip(std::io::Error),
    #[error("the SVG is not UTF-8 text")]
    NotText,
    #[error("the SVG does not parse: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("<{element}> refers to {href:?}, which is not embedded in the file")]
    ExternalReference { element: String, href: String },
    #[error("an embedded picture is {width} × {height} pixels, more than the preview decodes")]
    PictureTooLarge { width: usize, height: usize },
}

pub fn check_svg(data: Vec<u8>, max_len: u64, max_pixels: u64) -> Result<Vec<u8>, SvgError> {
    let data = if data.starts_with(&GZIP_MAGIC) {
        gunzip(&data, max_len)?
    } else {
        data
    };
    let text = std::str::from_utf8(&data).map_err(|_| SvgError::NotText)?;
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let document = roxmltree::Document::parse_with_options(text, options)?;
    for node in document.descendants().filter(roxmltree::Node::is_element) {
        let element = node.tag_name().name();
        if element != "image" && element != "feImage" {
            continue;
        }
        for href in node
            .attributes()
            .filter(|attribute| attribute.name() == "href")
            .map(|attribute| attribute.value())
        {
            check_href(element, href, max_pixels)?;
        }
    }
    Ok(data)
}

fn check_href(element: &str, href: &str, max_pixels: u64) -> Result<(), SvgError> {
    let external = || SvgError::ExternalReference {
        element: element.to_string(),
        href: href.chars().take(SHOWN_HREF_CHARS).collect(),
    };
    let url = data_url::DataUrl::process(href).map_err(|_| external())?;
    let mime = url.mime_type();
    match (mime.type_.as_str(), mime.subtype.as_str()) {
        ("image", "svg+xml") | ("text", "plain") => Err(external()),
        ("image", "png" | "jpeg" | "jpg" | "gif" | "webp") => {
            let Ok((picture, _)) = url.decode_to_vec() else {
                return Ok(());
            };
            match imagesize::blob_size(&picture) {
                Ok(size) if (size.width as u64).saturating_mul(size.height as u64) > max_pixels => {
                    Err(SvgError::PictureTooLarge {
                        width: size.width,
                        height: size.height,
                    })
                }
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

fn gunzip(data: &[u8], max_len: u64) -> Result<Vec<u8>, SvgError> {
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(max_len.saturating_add(1))
        .read_to_end(&mut unpacked)
        .map_err(SvgError::Gzip)?;
    if unpacked.len() as u64 > max_len {
        return Err(SvgError::TooLarge(max_len));
    }
    Ok(unpacked)
}
