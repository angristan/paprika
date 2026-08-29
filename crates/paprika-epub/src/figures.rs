use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as SourcePdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use pdf_oxide::PdfDocument;
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;

use super::buffer::BoundedBuffer;
use super::geometry::{expand_rect, intersect_rect, union_rect};
use super::images::account_asset;
use super::model::{ImagePlacement, PageImage};
use super::{
    EpubError, FIGURE_CROP_HEIGHT_POINTS, FIGURE_RENDER_SCALE, MAX_FIGURE_RENDER_EDGE,
    MAX_FIGURE_RENDER_PIXELS,
};

#[derive(Debug, Default)]
pub(super) struct FigureCrops {
    pub(super) images: Vec<PageImage>,
    pub(super) regions: Vec<Rect>,
    pub(super) text_exclusions: Vec<Rect>,
}

#[derive(Clone, Debug)]
struct FigureCaption {
    bbox: Rect,
    text: String,
    column: FigureColumn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FigureColumn {
    Left,
    Right,
    Full,
}

pub(super) fn page_may_have_figure_caption(spans: &[TextSpan]) -> bool {
    spans.iter().any(|span| {
        span.text.split_whitespace().any(|word| {
            let token = word.trim_matches(|character: char| !character.is_alphanumeric());
            token.eq_ignore_ascii_case("figure") || token.eq_ignore_ascii_case("fig")
        })
    })
}

pub(super) fn collect_figure_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    page_index: usize,
    image_bounds: &[Rect],
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<FigureCrops, EpubError> {
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
        warnings.push(format!(
            "Caption-based figure fallback for rotated page {} is unavailable.",
            page_index + 1
        ));
        return Ok(FigureCrops::default());
    }
    let Some(page) = source.pages().get(page_index) else {
        return Ok(FigureCrops::default());
    };
    let crop_box = page.intersected_crop_box();
    let (llx, lly, urx, ury) = (
        crop_box.x0 as f32,
        crop_box.y0 as f32,
        crop_box.x1 as f32,
        crop_box.y1 as f32,
    );
    let page_width = urx - llx;
    let page_height = ury - lly;
    if page_width <= 0.0 || page_height <= 0.0 {
        return Ok(FigureCrops::default());
    }
    let mut captions = find_figure_captions(document, page_index, llx, page_width);
    if captions.is_empty() {
        return Ok(FigureCrops::default());
    }
    captions.sort_by(|left, right| left.bbox.y.total_cmp(&right.bbox.y));

    let (base_width, base_height) = page.render_dimensions();
    let width = (base_width * FIGURE_RENDER_SCALE).ceil().max(1.0) as u32;
    let height = (base_height * FIGURE_RENDER_SCALE).ceil().max(1.0) as u32;
    if width > MAX_FIGURE_RENDER_EDGE
        || height > MAX_FIGURE_RENDER_EDGE
        || u64::from(width) * u64::from(height) > MAX_FIGURE_RENDER_PIXELS
        || width > u16::MAX as u32
        || height > u16::MAX as u32
    {
        warnings.push(format!(
            "Figure crops from page {} exceed the render memory limit and were skipped.",
            page_index + 1
        ));
        return Ok(FigureCrops::default());
    }
    let pixmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings {
            x_scale: FIGURE_RENDER_SCALE,
            y_scale: FIGURE_RENDER_SCALE,
            width: Some(width as u16),
            height: Some(height as u16),
            bg_color: WHITE,
        },
    );
    let rgba: Vec<u8> = bytemuck::cast_vec(pixmap.take_unpremultiplied());
    let Some(rendered) = image::RgbaImage::from_raw(width, height, rgba) else {
        warnings.push(format!(
            "Could not render figure fallback for page {}.",
            page_index + 1
        ));
        return Ok(FigureCrops::default());
    };

    let x_scale = width as f32 / page_width;
    let y_scale = height as f32 / page_height;
    let path_bounds: Vec<Rect> = document
        .extract_paths(page_index)
        .map(|paths| paths.into_iter().map(|path| path.bbox).collect())
        .unwrap_or_default();
    let mut images = Vec::with_capacity(captions.len());
    let mut regions = Vec::with_capacity(captions.len());
    let mut text_exclusions = Vec::with_capacity(captions.len());
    for (caption_index, caption) in captions.iter().enumerate() {
        let (crop_left, crop_right) = match caption.column {
            FigureColumn::Left => (llx + page_width * 0.055, llx + page_width * 0.49),
            FigureColumn::Right => (llx + page_width * 0.51, urx - page_width * 0.055),
            FigureColumn::Full => (llx + page_width * 0.055, urx - page_width * 0.055),
        };
        let crop_bottom = caption.bbox.y + caption.bbox.height + 2.0;
        let mut crop_top = (crop_bottom + FIGURE_CROP_HEIGHT_POINTS).min(ury - 20.0);
        if let Some(next_caption) = captions.iter().find(|other| {
            other.bbox.y > caption.bbox.y
                && (other.column == caption.column
                    || other.column == FigureColumn::Full
                    || caption.column == FigureColumn::Full)
        }) {
            crop_top = crop_top.min(next_caption.bbox.y - 8.0);
        }
        if crop_top - crop_bottom < 24.0 {
            continue;
        }

        let coarse_region = Rect::new(
            crop_left,
            crop_bottom,
            crop_right - crop_left,
            crop_top - crop_bottom,
        );
        let Some(graphic_region) = figure_graphic_region(coarse_region, &path_bounds, image_bounds)
        else {
            continue;
        };
        let x = ((coarse_region.x - llx) * x_scale).floor().max(0.0) as u32;
        let y = ((ury - coarse_region.y - coarse_region.height) * y_scale)
            .floor()
            .max(0.0) as u32;
        let crop_width = (coarse_region.width * x_scale)
            .ceil()
            .min(width.saturating_sub(x) as f32) as u32;
        let crop_height = (coarse_region.height * y_scale)
            .ceil()
            .min(height.saturating_sub(y) as f32) as u32;
        if crop_width < 80 || crop_height < 48 {
            continue;
        }

        let crop = image::imageops::crop_imm(&rendered, x, y, crop_width, crop_height).to_image();
        let remaining_bytes = max_bytes.saturating_sub(*total_bytes);
        let mut output = BoundedBuffer::new(remaining_bytes);
        let encode_result =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut output, image::ImageFormat::Png);
        if output.limit_exceeded {
            return Err(EpubError::AssetsTooLarge {
                limit: max_bytes / (1024 * 1024),
            });
        }
        if let Err(error) = encode_result {
            warnings.push(format!(
                "Could not encode figure {} from page {}: {error}",
                caption_index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = output.into_inner();
        account_asset(total_bytes, bytes.len(), max_bytes)?;
        regions.push(coarse_region);
        text_exclusions.push(graphic_region);
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-figure-{:02}.png",
                page_index + 1,
                caption_index + 1
            ),
            bytes,
            alt: caption.text.clone(),
            placement: caption
                .text
                .split_once(':')
                .map_or(ImagePlacement::EndOfPage, |(label, _)| {
                    ImagePlacement::Caption(format!("{}:", label.trim()))
                }),
        });
    }
    Ok(FigureCrops {
        images,
        regions,
        text_exclusions,
    })
}

fn find_figure_captions(
    document: &PdfDocument,
    page_index: usize,
    page_left: f32,
    page_width: f32,
) -> Vec<FigureCaption> {
    let Ok(lines) = document.extract_text_lines(page_index) else {
        return Vec::new();
    };
    let mut captions = Vec::new();
    for line in lines {
        for (index, word) in line.words.iter().enumerate() {
            if !word
                .text
                .trim_matches(|character: char| !character.is_alphabetic())
                .eq_ignore_ascii_case("figure")
            {
                continue;
            }
            let Some(number) = line.words.get(index + 1) else {
                continue;
            };
            let marker = number.text.trim();
            let number_text = marker.trim_end_matches(':');
            if !marker.ends_with(':') || number_text.parse::<u16>().is_err() {
                continue;
            }
            let caption_words = &line.words[index..];
            if caption_words.len() < 4 {
                continue;
            }
            let bbox = caption_words
                .iter()
                .skip(1)
                .fold(word.bbox, |bbox, item| union_rect(bbox, item.bbox));
            let center = bbox.x + bbox.width * 0.5;
            let page_center = page_left + page_width * 0.5;
            let center_distance = (center - page_center).abs();
            let column = if center_distance < page_width * 0.12 || bbox.width > page_width * 0.48 {
                FigureColumn::Full
            } else if center < page_center {
                FigureColumn::Left
            } else {
                FigureColumn::Right
            };
            captions.push(FigureCaption {
                bbox,
                text: caption_words
                    .iter()
                    .map(|item| item.text.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
                column,
            });
            break;
        }
    }
    captions
}

pub(super) fn figure_graphic_region(
    coarse_region: Rect,
    path_bounds: &[Rect],
    image_bounds: &[Rect],
) -> Option<Rect> {
    tighten_regions_from_graphics(&[coarse_region], path_bounds, image_bounds)
        .into_iter()
        .next()
}

pub(super) fn tighten_regions_from_graphics(
    coarse_regions: &[Rect],
    path_bounds: &[Rect],
    image_bounds: &[Rect],
) -> Vec<Rect> {
    coarse_regions
        .iter()
        .filter_map(|coarse| {
            let mut graphics: Option<Rect> = None;
            for candidate in path_bounds.iter().chain(image_bounds) {
                let size_is_local = candidate.width <= coarse.width * 1.2
                    && candidate.height <= coarse.height * 1.2;
                if !size_is_local || !rect_center_is_inside(*candidate, *coarse) {
                    continue;
                }
                let Some(clipped) = intersect_rect(*candidate, *coarse) else {
                    continue;
                };
                graphics = Some(graphics.map_or(clipped, |current| union_rect(current, clipped)));
            }
            let graphics = graphics?;
            (graphics.width >= 24.0 && graphics.height >= 24.0)
                .then(|| intersect_rect(expand_rect(graphics, 2.0), *coarse))
                .flatten()
        })
        .collect()
}

fn rect_center_is_inside(subject: Rect, region: Rect) -> bool {
    let center_x = subject.x + subject.width * 0.5;
    let center_y = subject.y + subject.height * 0.5;
    center_x >= region.x
        && center_x <= region.x + region.width
        && center_y >= region.y
        && center_y <= region.y + region.height
}
