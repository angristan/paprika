use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as SourcePdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use pdf_oxide::PdfDocument;
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;

use super::buffer::BoundedBuffer;
use super::geometry::{median_body_font_size, overlaps_any, visual_page_regions};
use super::images::account_asset;
use super::model::{ImagePlacement, PageImage};
use super::sanitization::strip_markup;
use super::{
    EpubError, FIGURE_RENDER_SCALE, FORMULA_HEAVY_MIN_MATH_CHARACTERS,
    FORMULA_HEAVY_MIN_MATH_SPANS, MAX_FIGURE_RENDER_EDGE, MAX_FIGURE_RENDER_PIXELS,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct PageVisualContext<'a> {
    pub(super) index: usize,
    pub(super) spans: &'a [TextSpan],
    pub(super) image_bounds: &'a [Rect],
}

pub(super) fn is_math_dense_candidate(spans: &[TextSpan], excluded_regions: &[Rect]) -> bool {
    let mut content_spans = 0usize;
    let mut math_spans = 0usize;
    let mut math_characters = 0usize;

    for span in spans.iter().filter(|span| {
        !span.text.trim().is_empty()
            && span.artifact_type.is_none()
            && span.rotation_degrees.abs() < 1.0
            && !overlaps_any(span.bbox, excluded_regions)
    }) {
        content_spans += 1;
        if is_math_span(span) {
            math_spans += 1;
            math_characters += span
                .text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
        }
    }

    math_spans >= FORMULA_HEAVY_MIN_MATH_SPANS
        && math_characters >= FORMULA_HEAVY_MIN_MATH_CHARACTERS
        && math_spans.saturating_mul(3) >= content_spans
}

pub(super) fn math_extraction_is_unreliable(
    spans: &[TextSpan],
    excluded_regions: &[Rect],
    extracted_html: &str,
) -> bool {
    let body_font_size = median_body_font_size(spans);
    let math_spans: Vec<&TextSpan> = spans
        .iter()
        .filter(|span| {
            span.artifact_type.is_none()
                && span.rotation_degrees.abs() < 1.0
                && !overlaps_any(span.bbox, excluded_regions)
                && is_math_span(span)
        })
        .collect();
    let source_syntax = math_spans
        .iter()
        .map(|span| math_syntax_count(&span.text))
        .sum::<usize>();
    let non_math_source_syntax = spans
        .iter()
        .filter(|span| {
            span.artifact_type.is_none()
                && span.rotation_degrees.abs() < 1.0
                && !overlaps_any(span.bbox, excluded_regions)
                && !is_math_span(span)
        })
        .map(|span| math_syntax_count(&span.text))
        .sum::<usize>();
    let extracted_syntax = math_syntax_count(&strip_markup(extracted_html));
    let retained_math_syntax = extracted_syntax.saturating_sub(non_math_source_syntax);
    let fragmented_spans = math_spans
        .iter()
        .filter(|span| {
            span.text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count()
                <= 2
        })
        .count();
    let script_spans = math_spans
        .iter()
        .filter(|span| span.font_size <= body_font_size * 0.82)
        .count();

    (source_syntax >= 6 && retained_math_syntax.saturating_mul(2) < source_syntax)
        || (fragmented_spans >= FORMULA_HEAVY_MIN_MATH_SPANS && script_spans >= 12)
}

pub(super) fn trustworthy_prose_html(html: &str) -> String {
    let mut blocks = Vec::new();
    for tag in ["p", "h1", "h2", "h3", "h4", "h5", "h6"] {
        let opening = format!("<{tag}");
        let closing = format!("</{tag}>");
        let mut search_from = 0usize;
        while let Some(relative_start) = html[search_from..].find(&opening) {
            let start = search_from + relative_start;
            let Some(relative_open_end) = html[start..].find('>') else {
                break;
            };
            let content_start = start + relative_open_end + 1;
            let Some(relative_close) = html[content_start..].find(&closing) else {
                break;
            };
            let end = content_start + relative_close + closing.len();
            let text = strip_markup(&html[content_start..content_start + relative_close]);
            let alphabetic = text
                .chars()
                .filter(|character| character.is_alphabetic())
                .count();
            let words = text
                .split_whitespace()
                .filter(|word| {
                    word.chars()
                        .filter(|character| character.is_alphabetic())
                        .count()
                        >= 2
                })
                .count();
            let minimum_words = if tag == "p" { 5 } else { 2 };
            if alphabetic >= 12
                && words >= minimum_words
                && math_syntax_count(&text) == 0
                && !text.contains('\u{fffd}')
            {
                blocks.push((start, end));
            }
            search_from = end;
        }
    }
    blocks.sort_unstable_by_key(|(start, _)| *start);

    let mut output = String::new();
    let mut previous_end = 0usize;
    for (start, end) in blocks {
        if start < previous_end {
            continue;
        }
        output.push_str(&html[start..end]);
        output.push('\n');
        previous_end = end;
    }
    output
}

fn math_syntax_count(text: &str) -> usize {
    text.chars()
        .filter(|character| {
            matches!(
                character,
                '=' | '+'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '−'
                    | '±'
                    | '×'
                    | '÷'
                    | '<'
                    | '>'
                    | '≤'
                    | '≥'
                    | '≠'
                    | '≈'
                    | '∝'
                    | '←'
                    | '→'
                    | '∑'
                    | '∏'
                    | '∫'
                    | '√'
                    | '∈'
                    | '∉'
                    | '⊂'
                    | '⊆'
                    | '∪'
                    | '∩'
            )
        })
        .count()
}

pub(super) fn collect_formula_page_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    context: PageVisualContext<'_>,
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<PageImage>, EpubError> {
    let PageVisualContext {
        index: page_index,
        spans,
        image_bounds,
    } = context;
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
        warnings.push(format!(
            "Visual math fallback for rotated page {} is unavailable; selectable text was retained where possible.",
            page_index + 1
        ));
        return Ok(Vec::new());
    }
    let Some(page) = source.pages().get(page_index) else {
        return Ok(Vec::new());
    };
    let crop_box = page.intersected_crop_box();
    let page_bounds = Rect::new(
        crop_box.x0 as f32,
        crop_box.y0 as f32,
        (crop_box.x1 - crop_box.x0) as f32,
        (crop_box.y1 - crop_box.y0) as f32,
    );
    if page_bounds.width <= 0.0 || page_bounds.height <= 0.0 {
        return Ok(Vec::new());
    }

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
            "Visual math fallback for page {} exceeds the render memory limit and was skipped.",
            page_index + 1
        ));
        return Ok(Vec::new());
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
            "Could not render visual math fallback for page {}.",
            page_index + 1
        ));
        return Ok(Vec::new());
    };

    let mut graphic_bounds: Vec<Rect> = document
        .extract_paths(page_index)
        .map(|paths| paths.into_iter().map(|path| path.bbox).collect())
        .unwrap_or_default();
    graphic_bounds.extend_from_slice(image_bounds);
    graphic_bounds.retain(|bbox| {
        bbox.width > 1.0
            && bbox.height > 1.0
            && bbox.width * bbox.height < page_bounds.width * page_bounds.height * 0.9
    });

    let regions = visual_page_regions(spans, page_bounds, &graphic_bounds);
    let x_scale = width as f32 / page_bounds.width;
    let y_scale = height as f32 / page_bounds.height;
    let mut images = Vec::with_capacity(regions.len());
    for (image_index, (region, label)) in regions.into_iter().enumerate() {
        let x = ((region.x - page_bounds.x) * x_scale).floor().max(0.0) as u32;
        let y = ((page_bounds.y + page_bounds.height - region.y - region.height) * y_scale)
            .floor()
            .max(0.0) as u32;
        let crop_width = (region.width * x_scale)
            .ceil()
            .min(width.saturating_sub(x) as f32) as u32;
        let crop_height = (region.height * y_scale)
            .ceil()
            .min(height.saturating_sub(y) as f32) as u32;
        if crop_width < 80 || crop_height < 80 {
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
                "Could not encode visual math fallback {} from page {}: {error}",
                image_index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = output.into_inner();
        account_asset(total_bytes, bytes.len(), max_bytes)?;
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-visual-math-{:02}.png",
                page_index + 1,
                image_index + 1
            ),
            bytes,
            alt: format!(
                "Visual fallback for source page {}, {}",
                page_index + 1,
                label
            ),
            placement: ImagePlacement::VisualPageFallback,
        });
    }
    Ok(images)
}

pub(super) fn is_math_span(span: &TextSpan) -> bool {
    let font = span.font_name.to_ascii_lowercase();
    [
        "math", "cmmi", "cmsy", "cmex", "txmi", "txsy", "txex", "msam", "msbm",
    ]
    .iter()
    .any(|marker| font.contains(marker))
        || span.text.chars().any(|character| {
            matches!(
                character,
                '∑' | '∏'
                    | '∫'
                    | '√'
                    | '≤'
                    | '≥'
                    | '≠'
                    | '≈'
                    | '∞'
                    | '∈'
                    | '∉'
                    | '⊂'
                    | '⊆'
                    | '∪'
                    | '∩'
                    | '∀'
                    | '∃'
                    | '⇒'
                    | '⇔'
                    | '→'
                    | '←'
            )
        })
}
