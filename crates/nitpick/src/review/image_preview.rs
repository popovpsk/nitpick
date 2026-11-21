use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use git_repo::Repo;
use gpui::{
    AnyElement, App, AppContext as _, Context, Image, ImageFormat, ImageSource, IntoElement,
    ObjectFit, ParentElement as _, RenderImage, SharedString, Styled as _, StyledImage as _,
    Subscription, SvgRenderer, Task, div, img, prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, h_flex, spinner::Spinner, v_flex};
use review_core::FileStatus;

use super::model::ReviewFile;
use super::{ReviewSource, ReviewView};
use crate::localization::tr;

const MAX_PREVIEW_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PREVIEW_PIXELS: u64 = 50_000_000;
const STACK_BELOW: f32 = 520.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Before,
    After,
}

enum Source {
    Blob(String),
    Disk(PathBuf),
}

#[derive(Clone, Copy)]
enum SizeUnit {
    Bytes,
    Kilobytes,
    Megabytes,
}

struct Caption {
    text: SharedString,
    unit: SizeUnit,
}

enum SideView {
    Loading,
    Ready {
        image: Arc<RenderImage>,
        caption: Caption,
    },
    TooLarge(Caption),
    Failed,
}

struct PreviewSide {
    which: Which,
    view: SideView,
    _task: Task<()>,
}

enum Loaded {
    Image(Arc<RenderImage>, u64),
    TooLarge(u64),
}

pub(super) struct ImagePreview {
    key: Option<(SharedString, String)>,
    generation: u64,
    sides: Vec<PreviewSide>,
    _release: Subscription,
}

impl ImagePreview {
    pub(super) fn new(cx: &mut Context<ReviewView>) -> Self {
        Self {
            key: None,
            generation: 0,
            sides: Vec::new(),
            _release: cx.on_release(|this: &mut ReviewView, cx: &mut App| {
                for image in this.image_preview.take_images() {
                    cx.drop_image(image, None);
                }
            }),
        }
    }

    fn take_images(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.sides)
            .into_iter()
            .filter_map(|side| match side.view {
                SideView::Ready { image, .. } => Some(image),
                _ => None,
            })
            .collect()
    }

    fn shows(&self, file: &ReviewFile) -> bool {
        self.key.as_ref().is_some_and(|(path, fingerprint)| {
            *path == file.path && *fingerprint == file.fingerprint
        })
    }
}

impl ReviewView {
    pub(super) fn request_image_preview(&mut self, cx: &mut Context<Self>) {
        let file = self
            .current_file
            .and_then(|ix| self.review.as_ref()?.files.get(ix));
        if let Some(file) = file
            && self.image_preview.shows(file)
        {
            return;
        }
        let sides = file.and_then(|file| preview_sides(file, &self.repo, &self.source));
        let key = file.map(|file| (file.path.clone(), file.fingerprint.clone()));
        let stale = self.image_preview.take_images();
        if !stale.is_empty() {
            cx.defer(move |cx| {
                for image in stale {
                    cx.drop_image(image, None);
                }
            });
        }
        self.image_preview.generation += 1;
        let Some(sides) = sides else {
            self.image_preview.key = None;
            return;
        };
        self.image_preview.key = key;
        let generation = self.image_preview.generation;
        let svg = cx.svg_renderer();
        self.image_preview.sides = sides
            .into_iter()
            .map(|(which, format, source)| {
                let repo = self.repo.clone();
                let svg = svg.clone();
                let task = cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { load(&repo, &source, format, svg) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.install_image(generation, which, result, cx)
                    });
                });
                PreviewSide {
                    which,
                    view: SideView::Loading,
                    _task: task,
                }
            })
            .collect();
        cx.notify();
    }

    fn install_image(
        &mut self,
        generation: u64,
        which: Which,
        result: anyhow::Result<Loaded>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.image_preview.generation {
            return;
        }
        let view = match result {
            Ok(Loaded::Image(image, bytes)) => {
                let size = image.size(0);
                let (amount, unit) = byte_size(bytes);
                let caption = Caption {
                    text: format!("{} × {} · {amount}", size.width.0, size.height.0).into(),
                    unit,
                };
                SideView::Ready { image, caption }
            }
            Ok(Loaded::TooLarge(bytes)) => {
                let (amount, unit) = byte_size(bytes);
                SideView::TooLarge(Caption {
                    text: amount.into(),
                    unit,
                })
            }
            Err(err) => {
                let path = self
                    .image_preview
                    .key
                    .as_ref()
                    .map(|(path, _)| path.clone());
                log::warn!("image preview {}: {err:#}", path.unwrap_or_default());
                SideView::Failed
            }
        };
        if let Some(side) = self
            .image_preview
            .sides
            .iter_mut()
            .find(|side| side.which == which)
        {
            side.view = view;
        }
        cx.notify();
    }

    pub(super) fn render_image_preview(
        &self,
        file: Option<&ReviewFile>,
        cx: &App,
    ) -> Option<AnyElement> {
        let file = file?;
        let preview = &self.image_preview;
        if !preview.shows(file) || preview.sides.is_empty() {
            return None;
        }
        let stack =
            preview.sides.len() > 1 && self.split_width > 0. && self.split_width < STACK_BELOW;
        Some(
            div()
                .size_full()
                .p_4()
                .flex()
                .gap_4()
                .when(stack, |el| el.flex_col())
                .children(preview.sides.iter().map(|side| render_side(side, cx)))
                .into_any_element(),
        )
    }
}

fn preview_sides(
    file: &ReviewFile,
    repo: &Repo,
    source: &ReviewSource,
) -> Option<Vec<(Which, ImageFormat, Source)>> {
    if !file.binary {
        return None;
    }
    let mut sides = Vec::with_capacity(2);
    if file.status != FileStatus::Added {
        let format = image_format(&file.old_path)?;
        sides.push((Which::Before, format, Source::Blob(file.old_blob.clone())));
    }
    if file.status != FileStatus::Deleted {
        let format = image_format(&file.new_path)?;
        let source = match source {
            ReviewSource::WorkingTree => Source::Disk(repo.root().join(file.new_path.as_ref())),
            _ => Source::Blob(file.new_blob.clone()),
        };
        sides.push((Which::After, format, source));
    }
    (!sides.is_empty()).then_some(sides)
}

fn image_format(path: &str) -> Option<ImageFormat> {
    let extension = Path::new(path).extension()?.to_str()?;
    [
        ("png", ImageFormat::Png),
        ("jpg", ImageFormat::Jpeg),
        ("jpeg", ImageFormat::Jpeg),
        ("gif", ImageFormat::Gif),
        ("webp", ImageFormat::Webp),
        ("bmp", ImageFormat::Bmp),
        ("svg", ImageFormat::Svg),
        ("ico", ImageFormat::Ico),
        ("tif", ImageFormat::Tiff),
        ("tiff", ImageFormat::Tiff),
    ]
    .into_iter()
    .find(|(name, _)| extension.eq_ignore_ascii_case(name))
    .map(|(_, format)| format)
}

fn load(
    repo: &Repo,
    source: &Source,
    format: ImageFormat,
    svg: SvgRenderer,
) -> anyhow::Result<Loaded> {
    let size = match source {
        Source::Blob(id) => repo.blob_size(id)?,
        Source::Disk(path) => std::fs::metadata(path)?.len(),
    };
    if size > MAX_PREVIEW_BYTES {
        return Ok(Loaded::TooLarge(size));
    }
    let bytes = match source {
        Source::Blob(id) => repo.blob(id)?,
        Source::Disk(path) => read_regular_file(path, MAX_PREVIEW_BYTES)?,
    };
    let length = bytes.len() as u64;
    if length > MAX_PREVIEW_BYTES {
        return Ok(Loaded::TooLarge(length));
    }
    let bytes = if format == ImageFormat::Svg {
        review_core::check_svg(bytes, MAX_PREVIEW_BYTES)?
    } else {
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()?
            .into_dimensions()?;
        if u64::from(width) * u64::from(height) > MAX_PREVIEW_PIXELS {
            return Ok(Loaded::TooLarge(length));
        }
        bytes
    };
    let (format, bytes) = match format {
        ImageFormat::Gif | ImageFormat::Webp => (ImageFormat::Png, first_frame_png(&bytes)?),
        _ => (format, bytes),
    };
    let image = Image::from_bytes(format, bytes).to_image_data(svg)?;
    Ok(Loaded::Image(image, length))
}

pub(super) fn read_regular_file(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn first_frame_png(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let frame = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    let mut png = std::io::Cursor::new(Vec::new());
    frame.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

fn byte_size(bytes: u64) -> (String, SizeUnit) {
    const KB: u64 = 1024;
    if bytes < KB {
        (bytes.to_string(), SizeUnit::Bytes)
    } else if bytes < KB * KB {
        (
            format!("{:.1}", bytes as f64 / KB as f64),
            SizeUnit::Kilobytes,
        )
    } else {
        (
            format!("{:.1}", bytes as f64 / (KB * KB) as f64),
            SizeUnit::Megabytes,
        )
    }
}

fn render_side(side: &PreviewSide, cx: &App) -> AnyElement {
    let s = tr();
    let theme = cx.theme();
    let (label, label_color) = match side.which {
        Which::Before => (s.image_before.clone(), theme.danger),
        Which::After => (s.image_after.clone(), theme.success),
    };
    let unit = |unit: SizeUnit| match unit {
        SizeUnit::Bytes => s.size_bytes.clone(),
        SizeUnit::Kilobytes => s.size_kb.clone(),
        SizeUnit::Megabytes => s.size_mb.clone(),
    };
    let message = |text: SharedString| {
        div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element()
    };
    let (body, caption) = match &side.view {
        SideView::Loading => (Spinner::new().into_any_element(), None),
        SideView::Ready { image, caption } => (
            img(ImageSource::Render(image.clone()))
                .size_full()
                .object_fit(ObjectFit::ScaleDown)
                .into_any_element(),
            Some(caption),
        ),
        SideView::TooLarge(caption) => (message(s.image_too_large.clone()), Some(caption)),
        SideView::Failed => (message(s.image_failed.clone()), None),
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(label_color)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .p_2()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(body),
        )
        .child(
            h_flex()
                .h_4()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .when_some(caption, |el, caption| {
                    el.child(caption.text.clone()).child(unit(caption.unit))
                }),
        )
        .into_any_element()
}

#[cfg(test)]
#[path = "image_preview_tests.rs"]
mod tests;
