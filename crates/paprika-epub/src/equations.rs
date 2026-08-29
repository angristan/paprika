use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as SourcePdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use pdf_oxide::PdfDocument;
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;

use super::buffer::BoundedBuffer;
use super::geometry::{
    SpanComponent, expand_rect, horizontal_span_components, intersect_rect, local_column_bounds,
    median_body_font_size, overlap_fraction, overlaps_any, page_has_two_text_columns,
    rect_vertical_gap, rects_intersect, union_rect,
};
use super::images::account_asset;
use super::math::{PageVisualContext, is_math_span};
use super::model::{ImagePlacement, PageImage};
use super::{EpubError, FIGURE_RENDER_SCALE, MAX_FIGURE_RENDER_EDGE, MAX_FIGURE_RENDER_PIXELS};

#[derive(Debug, Default)]
pub(super) struct EquationCrops {
    pub(super) images: Vec<PageImage>,
    pub(super) text_exclusions: Vec<Rect>,
    pub(super) asset_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AssetBudget {
    pub(super) used: usize,
    pub(super) maximum: usize,
}

#[derive(Clone, Debug)]
pub(super) struct EquationPlan {
    pub(super) render_bbox: Rect,
    pub(super) exclusion_rects: Vec<Rect>,
    pub(super) anchor: String,
}

pub(super) fn collect_equation_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    context: PageVisualContext<'_>,
    excluded_regions: &[Rect],
    asset_budget: AssetBudget,
    warnings: &mut Vec<String>,
) -> Result<EquationCrops, EpubError> {
    let PageVisualContext {
        index: page_index,
        spans,
        image_bounds: image_regions,
    } = context;
    let Some(page) = source.pages().get(page_index) else {
        return Ok(EquationCrops::default());
    };
    let crop_box = page.intersected_crop_box();
    let page_bounds = Rect::new(
        crop_box.x0 as f32,
        crop_box.y0 as f32,
        (crop_box.x1 - crop_box.x0) as f32,
        (crop_box.y1 - crop_box.y0) as f32,
    );
    if page_bounds.width <= 0.0 || page_bounds.height <= 0.0 {
        return Ok(EquationCrops::default());
    }

    let table_regions: Vec<Rect> = document
        .extract_tables(page_index)
        .map(|tables| tables.into_iter().filter_map(|table| table.bbox).collect())
        .unwrap_or_default();
    let mut veto_regions =
        Vec::with_capacity(excluded_regions.len() + table_regions.len() + image_regions.len());
    veto_regions.extend_from_slice(excluded_regions);
    veto_regions.extend(table_regions);
    veto_regions.extend_from_slice(image_regions);
    let plans = find_display_equations(spans, page_bounds, &veto_regions);
    if plans.is_empty() {
        return Ok(EquationCrops::default());
    }
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
        warnings.push(format!(
            "Equation fallbacks for rotated page {} are unavailable; the extracted equation text was retained.",
            page_index + 1
        ));
        return Ok(EquationCrops::default());
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
            "Equation crops from page {} exceed the render memory limit and were skipped.",
            page_index + 1
        ));
        return Ok(EquationCrops::default());
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
            "Could not render equation fallback for page {}.",
            page_index + 1
        ));
        return Ok(EquationCrops::default());
    };

    let x_scale = width as f32 / page_bounds.width;
    let y_scale = height as f32 / page_bounds.height;
    let mut images = Vec::with_capacity(plans.len());
    let mut text_exclusions = Vec::new();
    let mut cumulative_asset_bytes = asset_budget.used;
    let mut equation_asset_bytes = 0usize;
    for (index, plan) in plans.into_iter().enumerate() {
        let x = ((plan.render_bbox.x - page_bounds.x) * x_scale)
            .floor()
            .max(0.0) as u32;
        let y =
            ((page_bounds.y + page_bounds.height - plan.render_bbox.y - plan.render_bbox.height)
                * y_scale)
                .floor()
                .max(0.0) as u32;
        let crop_width = (plan.render_bbox.width * x_scale)
            .ceil()
            .min(width.saturating_sub(x) as f32) as u32;
        let crop_height = (plan.render_bbox.height * y_scale)
            .ceil()
            .min(height.saturating_sub(y) as f32) as u32;
        if crop_width < 32 || crop_height < 16 {
            continue;
        }

        let crop = image::imageops::crop_imm(&rendered, x, y, crop_width, crop_height).to_image();
        let remaining_bytes = asset_budget.maximum.saturating_sub(cumulative_asset_bytes);
        let mut output = BoundedBuffer::new(remaining_bytes);
        let encode_result =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut output, image::ImageFormat::Png);
        if output.limit_exceeded {
            return Err(EpubError::AssetsTooLarge {
                limit: asset_budget.maximum / (1024 * 1024),
            });
        }
        if let Err(error) = encode_result {
            warnings.push(format!(
                "Could not encode equation {} from page {}: {error}",
                index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = output.into_inner();
        account_asset(
            &mut cumulative_asset_bytes,
            bytes.len(),
            asset_budget.maximum,
        )?;
        equation_asset_bytes =
            equation_asset_bytes
                .checked_add(bytes.len())
                .ok_or(EpubError::AssetsTooLarge {
                    limit: asset_budget.maximum / (1024 * 1024),
                })?;
        text_exclusions.extend(plan.exclusion_rects);
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-equation-{:02}.png",
                page_index + 1,
                index + 1
            ),
            bytes,
            alt: if is_equation_number(&plan.anchor) {
                format!(
                    "Display equation {} from source page {}",
                    plan.anchor,
                    page_index + 1
                )
            } else {
                format!("Display equation from source page {}", page_index + 1)
            },
            placement: ImagePlacement::EquationAnchor(plan.anchor),
        });
    }

    Ok(EquationCrops {
        images,
        text_exclusions,
        asset_bytes: equation_asset_bytes,
    })
}

pub(super) fn find_display_equations(
    spans: &[TextSpan],
    page_bounds: Rect,
    veto_regions: &[Rect],
) -> Vec<EquationPlan> {
    let mut plans = find_numbered_equations(spans, page_bounds, veto_regions);
    plans.extend(find_unnumbered_equations(
        spans,
        page_bounds,
        veto_regions,
        &plans,
    ));
    plans.sort_by(|left, right| right.render_bbox.y.total_cmp(&left.render_bbox.y));
    plans
}

fn find_numbered_equations(
    spans: &[TextSpan],
    page_bounds: Rect,
    veto_regions: &[Rect],
) -> Vec<EquationPlan> {
    let body_font_size = median_body_font_size(spans);
    let two_columns = page_has_two_text_columns(spans, page_bounds);
    let page_midpoint = page_bounds.x + page_bounds.width * 0.5;
    let equation_numbers: Vec<&TextSpan> = spans
        .iter()
        .filter(|span| is_equation_number(&span.text))
        .collect();
    let mut plans = Vec::new();

    for number in &equation_numbers {
        let number_center = number.bbox.x + number.bbox.width * 0.5;
        let (column_left, column_right) = if two_columns {
            if number_center < page_midpoint {
                (page_bounds.x, page_midpoint)
            } else {
                (page_midpoint, page_bounds.x + page_bounds.width)
            }
        } else {
            (page_bounds.x, page_bounds.x + page_bounds.width)
        };
        let column_width = column_right - column_left;
        if number_center < column_left + column_width * 0.78
            || overlaps_any(number.bbox, veto_regions)
        {
            continue;
        }

        let number_center_y = number.bbox.y + number.bbox.height * 0.5;
        let vertical_radius = body_font_size * 1.6;
        let formula_spans: Vec<&TextSpan> = spans
            .iter()
            .filter(|span| !std::ptr::eq(*span, *number))
            .filter(|span| {
                let center_x = span.bbox.x + span.bbox.width * 0.5;
                let center_y = span.bbox.y + span.bbox.height * 0.5;
                let distance = (center_y - number_center_y).abs();
                let belongs_to_this_number = !equation_numbers.iter().any(|other| {
                    if std::ptr::eq(*other, *number) {
                        return false;
                    }
                    let other_center_x = other.bbox.x + other.bbox.width * 0.5;
                    let other_center_y = other.bbox.y + other.bbox.height * 0.5;
                    other_center_x >= column_left + column_width * 0.78
                        && other_center_x <= column_right
                        && (center_y - other_center_y).abs() < distance
                });
                center_x >= column_left
                    && center_x < number.bbox.x - body_font_size * 0.4
                    && distance <= vertical_radius
                    && belongs_to_this_number
                    && !overlaps_any(span.bbox, veto_regions)
            })
            .collect();
        if formula_spans.len() < 2
            || formula_spans.iter().any(|span| is_mixed_prose_span(span))
            || !formula_spans.iter().any(|span| is_math_span(span))
            || !formula_spans
                .iter()
                .any(|span| contains_relation_operator(&span.text))
        {
            continue;
        }

        let formula_bbox = formula_spans
            .iter()
            .skip(1)
            .fold(formula_spans[0].bbox, |bbox, span| {
                union_rect(bbox, span.bbox)
            });
        if formula_bbox.width < body_font_size * 4.0 || formula_bbox.height < body_font_size * 0.7 {
            continue;
        }
        let render_bbox = intersect_rect(
            expand_rect(union_rect(formula_bbox, number.bbox), 3.0),
            page_bounds,
        )
        .unwrap_or_else(|| union_rect(formula_bbox, number.bbox));
        plans.push(EquationPlan {
            render_bbox,
            exclusion_rects: formula_spans
                .into_iter()
                .map(|span| expand_rect(span.bbox, 0.5))
                .collect(),
            anchor: number.text.trim().to_string(),
        });
    }

    plans
}

fn find_unnumbered_equations(
    spans: &[TextSpan],
    page_bounds: Rect,
    veto_regions: &[Rect],
    numbered: &[EquationPlan],
) -> Vec<EquationPlan> {
    if spans.iter().any(|span| {
        let text = span.text.trim_start();
        text.starts_with("Algorithm ") || text.starts_with("Listing ")
    }) {
        return Vec::new();
    }

    let body_font_size = median_body_font_size(spans);
    let two_columns = page_has_two_text_columns(spans, page_bounds);
    let components = horizontal_span_components(spans, body_font_size);
    let formula_components: Vec<&SpanComponent> = components
        .iter()
        .filter(|component| {
            is_display_formula_component(component, spans, page_bounds, two_columns, veto_regions)
                && !numbered
                    .iter()
                    .any(|plan| overlap_fraction(component.bbox, plan.render_bbox) >= 0.25)
        })
        .collect();
    let mut consumed = vec![false; formula_components.len()];
    let mut plans = Vec::new();

    for (index, primary) in formula_components.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        let (column_left, column_right) =
            local_column_bounds(primary.bbox, page_bounds, two_columns);
        let mut members = vec![index];
        for (other_index, other) in formula_components.iter().enumerate().skip(index + 1) {
            if consumed[other_index] {
                continue;
            }
            let same_column =
                other.bbox.x + other.bbox.width > column_left && other.bbox.x < column_right;
            let vertical_gap = rect_vertical_gap(primary.bbox, other.bbox);
            if same_column && vertical_gap <= body_font_size * 1.1 {
                members.push(other_index);
            }
        }

        let mut span_indices = Vec::new();
        let mut render_bbox = primary.bbox;
        for member in &members {
            consumed[*member] = true;
            let component = formula_components[*member];
            render_bbox = union_rect(render_bbox, component.bbox);
            span_indices.extend(component.indices.iter().copied());
        }
        let attachment_region = expand_rect(render_bbox, body_font_size * 0.6);
        for (span_index, span) in spans.iter().enumerate() {
            let center_x = span.bbox.x + span.bbox.width * 0.5;
            if center_x >= column_left
                && center_x <= column_right
                && (is_math_span(span) || span.font_size <= body_font_size * 0.82)
                && !is_mixed_prose_span(span)
                && rects_intersect(span.bbox, attachment_region)
            {
                render_bbox = union_rect(render_bbox, span.bbox);
                span_indices.push(span_index);
            }
        }
        span_indices.sort_unstable();
        span_indices.dedup();
        if !is_vertically_isolated(
            render_bbox,
            &span_indices,
            spans,
            column_left,
            column_right,
            body_font_size,
        ) {
            continue;
        }

        let Some(anchor_index) = span_indices.iter().copied().find(|span_index| {
            let text = spans[*span_index].text.trim();
            text.len() >= 3
                && text.chars().any(char::is_alphabetic)
                && !contains_relation_operator(text)
        }) else {
            continue;
        };
        let anchor = spans[anchor_index].text.trim().to_string();
        if anchor.split_whitespace().count() > 2 {
            continue;
        }
        let exclusion_rects = span_indices
            .into_iter()
            .filter(|span_index| *span_index != anchor_index)
            .map(|span_index| expand_rect(spans[span_index].bbox, 0.5))
            .collect();
        let render_bbox =
            intersect_rect(expand_rect(render_bbox, 3.0), page_bounds).unwrap_or(render_bbox);
        plans.push(EquationPlan {
            render_bbox,
            exclusion_rects,
            anchor,
        });
    }

    plans
}

fn is_display_formula_component(
    component: &SpanComponent,
    spans: &[TextSpan],
    page_bounds: Rect,
    two_columns: bool,
    veto_regions: &[Rect],
) -> bool {
    if overlaps_any(component.bbox, veto_regions) {
        return false;
    }
    let body_font_size = median_body_font_size(spans);
    let (column_left, column_right) = local_column_bounds(component.bbox, page_bounds, two_columns);
    let column_width = column_right - column_left;
    let left_margin = component.bbox.x - column_left;
    let right_margin = column_right - component.bbox.x - component.bbox.width;
    if component.bbox.width < body_font_size * 4.0
        || component.bbox.width > column_width * 0.78
        || left_margin < column_width * 0.1
        || right_margin < column_width * 0.1
    {
        return false;
    }

    let mut total_characters = 0usize;
    let mut math_characters = 0usize;
    let mut alphabetic_words = 0usize;
    let mut has_relation = false;
    for index in &component.indices {
        let span = &spans[*index];
        if span.heading_level.is_some() || span.is_monospace || is_veto_label(&span.text) {
            return false;
        }
        let character_count = span
            .text
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        total_characters += character_count;
        if is_math_span(span) {
            math_characters += character_count;
        }
        if !is_math_span(span) {
            alphabetic_words += span
                .text
                .split_whitespace()
                .filter(|word| {
                    word.chars()
                        .filter(|character| character.is_alphabetic())
                        .count()
                        >= 2
                })
                .count();
        }
        has_relation |= contains_relation_operator(&span.text);
    }
    has_relation
        && alphabetic_words <= 8
        && total_characters >= 6
        && math_characters * 5 >= total_characters
}

fn is_vertically_isolated(
    bbox: Rect,
    member_indices: &[usize],
    spans: &[TextSpan],
    column_left: f32,
    column_right: f32,
    body_font_size: f32,
) -> bool {
    let minimum_gap = body_font_size * 0.75;
    spans.iter().enumerate().all(|(index, span)| {
        if member_indices.binary_search(&index).is_ok() {
            return true;
        }
        let center_x = span.bbox.x + span.bbox.width * 0.5;
        let horizontal_overlap =
            (bbox.x + bbox.width).min(span.bbox.x + span.bbox.width) - bbox.x.max(span.bbox.x);
        center_x < column_left
            || center_x > column_right
            || horizontal_overlap <= 0.0
            || rect_vertical_gap(bbox, span.bbox) >= minimum_gap
    })
}

fn is_veto_label(text: &str) -> bool {
    let text = text.trim_start();
    [
        "Figure ",
        "Fig. ",
        "Table ",
        "Algorithm ",
        "Listing ",
        "Input:",
        "Output:",
        "parameter:",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn is_equation_number(text: &str) -> bool {
    let trimmed = text.trim();
    let Some(inner) = trimmed
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    !inner.is_empty()
        && inner.len() <= 6
        && inner
            .chars()
            .all(|character| character.is_ascii_digit() || character.is_ascii_lowercase())
        && inner.chars().any(|character| character.is_ascii_digit())
}

fn contains_relation_operator(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '∝' | '←' | '→'
        )
    })
}

fn is_mixed_prose_span(span: &TextSpan) -> bool {
    let words = span
        .text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count();
    words >= 5 && !is_math_span(span)
}
