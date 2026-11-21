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
}

pub fn check_svg(data: Vec<u8>, max_len: u64) -> Result<Vec<u8>, SvgError> {
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
            if !embedded_picture(href) {
                return Err(SvgError::ExternalReference {
                    element: element.to_string(),
                    href: href.chars().take(SHOWN_HREF_CHARS).collect(),
                });
            }
        }
    }
    Ok(data)
}

fn embedded_picture(href: &str) -> bool {
    data_url::DataUrl::process(href).is_ok_and(|url| {
        let mime = url.mime_type();
        !matches!(
            (mime.type_.as_str(), mime.subtype.as_str()),
            ("image", "svg+xml") | ("text", "plain")
        )
    })
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
