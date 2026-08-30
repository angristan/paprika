use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;

use super::math::is_math_span;

#[derive(Clone, Debug)]
pub(super) struct SpanComponent {
    pub(super) indices: Vec<usize>,
    pub(super) bbox: Rect,
}

#[derive(Clone, Debug)]
pub(super) struct ReconstructedTitle {
    pub(super) text: String,
    pub(super) bbox: Rect,
}

pub(super) const MAX_TITLE_SPANS: usize = 4_096;

/// Recover a title that column-aware extraction split at the page midpoint.
///
/// PDF text order is often unrelated to visual order. In two-column papers, a
/// centered title can therefore become one heading at the start of the left
/// column and another near the end of the right column. The first semantic
/// heading remains the anchor: geometry may only extend that exact text along
/// the same display-sized line cluster, never replace it with an unrelated
/// large label.
pub(super) fn reconstruct_document_title(
    spans: &[TextSpan],
    page_bounds: Rect,
    extracted_heading: &str,
    continuation_headings: &[String],
    expected_document_title: &str,
) -> Option<ReconstructedTitle> {
    let extracted_heading = normalize_title_text(extracted_heading);
    let expected_document_title = normalize_title_text(expected_document_title);
    let page_right = page_bounds.x + page_bounds.width;
    let page_top = page_bounds.y + page_bounds.height;
    if extracted_heading.is_empty()
        || !page_bounds.x.is_finite()
        || !page_bounds.y.is_finite()
        || !page_right.is_finite()
        || !page_top.is_finite()
        || page_bounds.width <= 0.0
        || page_bounds.height <= 0.0
    {
        return None;
    }

    let page_top_threshold = page_bounds.y + page_bounds.height * 0.6;
    let maximum_font_size = spans
        .iter()
        .filter(|span| {
            title_span_is_positioned(span, page_bounds, page_top_threshold)
                && span.text.chars().any(char::is_alphanumeric)
        })
        .map(|span| span.font_size)
        .max_by(f32::total_cmp)?;
    let minimum_font_size = (maximum_font_size * 0.88).max(11.0);

    let mut indices: Vec<usize> = (0..spans.len())
        .filter(|index| {
            let span = &spans[*index];
            title_span_is_positioned(span, page_bounds, page_top_threshold)
                && span.font_size >= minimum_font_size
        })
        .take(MAX_TITLE_SPANS + 1)
        .collect();
    if indices.len() > MAX_TITLE_SPANS {
        return None;
    }
    indices.sort_by(|left, right| {
        spans[*right]
            .bbox
            .y
            .total_cmp(&spans[*left].bbox.y)
            .then(spans[*left].bbox.x.total_cmp(&spans[*right].bbox.x))
    });

    let baseline_tolerance = maximum_font_size * 0.45;
    let mut rows: Vec<(f32, Vec<usize>)> = Vec::new();
    for index in indices {
        let center_y = spans[index].bbox.y + spans[index].bbox.height * 0.5;
        if let Some((baseline, row)) = rows.last_mut()
            && (center_y - *baseline).abs() <= baseline_tolerance
        {
            let count = row.len() as f32;
            *baseline = (*baseline * count + center_y) / (count + 1.0);
            row.push(index);
        } else {
            rows.push((center_y, vec![index]));
        }
    }
    rows.sort_by(|left, right| right.0.total_cmp(&left.0));

    let runs: Vec<(f32, Vec<TitleRun>)> = rows
        .into_iter()
        .map(|(baseline, row)| (baseline, title_runs_for_row(row, spans, maximum_font_size)))
        .collect();
    let mut matches = Vec::new();
    for (row_index, (_, row_runs)) in runs.iter().enumerate() {
        for (run_index, run) in row_runs.iter().enumerate() {
            if run.text == extracted_heading
                || run
                    .text
                    .strip_prefix(&extracted_heading)
                    .is_some_and(|suffix| suffix.starts_with(' '))
            {
                matches.push((row_index, run_index));
            }
        }
    }
    let [(start_row, start_run)] = matches.as_slice() else {
        return None;
    };

    let first = &runs[*start_row].1[*start_run];
    let mut text = first.text.clone();
    let mut bbox = first.bbox;
    let mut previous_baseline = runs[*start_row].0;
    // A lower row can be indistinguishable from an equally styled byline.
    // Only use one when the selected document title independently confirms the
    // reconstructed prefix; same-line repair does not need that extra cue.
    let continuation_rows = if first.text != extracted_heading
        && expected_document_title.starts_with(&first.text)
        && expected_document_title != first.text
    {
        &runs[*start_row + 1..]
    } else {
        &runs[0..0]
    };
    for (baseline, row_runs) in continuation_rows {
        if previous_baseline - *baseline > maximum_font_size * 1.2 {
            break;
        }
        let Some(continuation) = row_runs
            .iter()
            .filter(|run| {
                (run.font_size - first.font_size).abs() <= first.font_size * 0.03
                    && title_runs_align(bbox, run.bbox, maximum_font_size)
                    && continuation_headings.iter().any(|heading| {
                        let heading = normalize_title_text(heading);
                        run.text == heading
                            || run
                                .text
                                .strip_prefix(&heading)
                                .is_some_and(|suffix| suffix.starts_with(' '))
                    })
                    && expected_document_title
                        .starts_with(&normalize_title_text(&format!("{text} {}", run.text)))
            })
            .max_by_key(|run| {
                run.text
                    .chars()
                    .filter(|character| character.is_alphanumeric())
                    .count()
            })
        else {
            break;
        };
        if continuation.text.is_empty() {
            break;
        }
        text.push(' ');
        text.push_str(&continuation.text);
        bbox = union_rect(bbox, continuation.bbox);
        previous_baseline = *baseline;
    }

    let text = normalize_title_text(&text);
    if text.len() > 200
        || !text.starts_with(&extracted_heading)
        || (text != first.text && text != expected_document_title)
    {
        return None;
    }
    Some(ReconstructedTitle {
        text,
        bbox: intersect_rect(expand_rect(bbox, 1.0), page_bounds)?,
    })
}

#[derive(Clone, Debug)]
struct TitleRun {
    text: String,
    bbox: Rect,
    font_size: f32,
}

fn title_span_is_positioned(span: &TextSpan, page_bounds: Rect, page_top_threshold: f32) -> bool {
    let right = span.bbox.x + span.bbox.width;
    let top = span.bbox.y + span.bbox.height;
    span.rotation_degrees.abs() < 1.0
        && span.artifact_type.is_none()
        && span.font_size.is_finite()
        && span.font_size > 0.0
        && span.bbox.x.is_finite()
        && span.bbox.y.is_finite()
        && span.bbox.width.is_finite()
        && span.bbox.height.is_finite()
        && right.is_finite()
        && top.is_finite()
        && span.bbox.width >= 0.0
        && span.bbox.width <= page_bounds.width
        && span.bbox.height > 0.0
        && span.bbox.height <= page_bounds.height * 0.2
        && span.bbox.x >= page_bounds.x - 1.0
        && right <= page_bounds.x + page_bounds.width + 1.0
        && span.bbox.y >= page_bounds.y - 1.0
        && top <= page_bounds.y + page_bounds.height + 1.0
        && top >= page_top_threshold
        && !span.text.is_empty()
}

fn title_runs_for_row(
    mut indices: Vec<usize>,
    spans: &[TextSpan],
    maximum_font_size: f32,
) -> Vec<TitleRun> {
    indices.sort_by(|left, right| spans[*left].bbox.x.total_cmp(&spans[*right].bbox.x));
    let mut groups = Vec::<Vec<usize>>::new();
    let mut current = Vec::new();
    let mut right_edge = f32::NEG_INFINITY;
    for index in indices {
        let span = &spans[index];
        if !current.is_empty() && span.bbox.x - right_edge > maximum_font_size * 0.9 {
            groups.push(std::mem::take(&mut current));
        }
        right_edge = right_edge.max(span.bbox.x + span.bbox.width);
        current.push(index);
    }
    if !current.is_empty() {
        groups.push(current);
    }

    groups
        .into_iter()
        .filter_map(|indices| {
            let text = title_run_text(&indices, spans);
            (text
                .chars()
                .filter(|character| character.is_alphanumeric())
                .count()
                >= 2)
                .then(|| {
                    let font_size = indices
                        .iter()
                        .map(|index| spans[*index].font_size)
                        .max_by(f32::total_cmp)
                        .unwrap_or(maximum_font_size);
                    let bbox = component_from_indices(indices, spans).bbox;
                    TitleRun {
                        text,
                        bbox,
                        font_size,
                    }
                })
        })
        .collect()
}

fn title_run_text(indices: &[usize], spans: &[TextSpan]) -> String {
    let mut text = String::new();
    let mut previous_right = None;
    let mut previous_font_size = 0.0f32;
    for index in indices {
        let span = &spans[*index];
        if let Some(right) = previous_right
            && !text.ends_with(char::is_whitespace)
            && !span.text.starts_with(char::is_whitespace)
            && span.bbox.x - right > previous_font_size.max(span.font_size) * 0.08
        {
            text.push(' ');
        }
        text.push_str(&span.text);
        previous_right = Some(span.bbox.x + span.bbox.width);
        previous_font_size = span.font_size;
    }
    normalize_title_text(&text)
}

fn normalize_title_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn title_runs_align(left: Rect, right: Rect, font_size: f32) -> bool {
    let overlap = (left.x + left.width).min(right.x + right.width) - left.x.max(right.x);
    let center_distance = ((left.x + left.width * 0.5) - (right.x + right.width * 0.5)).abs();
    overlap > 0.0 || center_distance <= left.width.max(right.width) * 0.35 + font_size
}

pub(super) fn visual_page_regions(
    spans: &[TextSpan],
    page_bounds: Rect,
    graphic_bounds: &[Rect],
) -> Vec<(Rect, String)> {
    let two_columns = page_has_two_text_columns(spans, page_bounds)
        && !has_full_width_content(spans, page_bounds, graphic_bounds);
    let columns = if two_columns {
        let midpoint = page_bounds.x + page_bounds.width * 0.5;
        // Overlap the center instead of deleting a presumed gutter. This also
        // preserves equations and headings that cross the detected columns.
        let overlap = page_bounds.width * 0.015;
        vec![
            Rect::new(
                page_bounds.x,
                page_bounds.y,
                midpoint + overlap - page_bounds.x,
                page_bounds.height,
            ),
            Rect::new(
                midpoint - overlap,
                page_bounds.y,
                page_bounds.x + page_bounds.width - midpoint + overlap,
                page_bounds.height,
            ),
        ]
    } else {
        vec![page_bounds]
    };

    let mut regions = Vec::new();
    for (column_index, column) in columns.into_iter().enumerate() {
        let parts: Vec<Rect> = split_visual_column(column, spans, graphic_bounds)
            .into_iter()
            .filter(|part| visual_region_has_content(*part, spans, graphic_bounds))
            .collect();
        let part_count = parts.len();
        for (part_index, part) in parts.into_iter().enumerate() {
            let label = if two_columns {
                format!(
                    "{} column, part {} of {}",
                    if column_index == 0 { "left" } else { "right" },
                    part_index + 1,
                    part_count
                )
            } else {
                format!("part {} of {}", part_index + 1, part_count)
            };
            regions.push((part, label));
        }
    }
    regions
}

fn visual_region_has_content(region: Rect, spans: &[TextSpan], graphic_bounds: &[Rect]) -> bool {
    spans.iter().any(|span| {
        !span.text.trim().is_empty()
            && materially_overlaps_horizontally(span.bbox, region)
            && rects_intersect(span.bbox, region)
    }) || graphic_bounds.iter().any(|bbox| {
        materially_overlaps_horizontally(*bbox, region) && rects_intersect(*bbox, region)
    })
}

fn has_full_width_content(spans: &[TextSpan], page_bounds: Rect, graphic_bounds: &[Rect]) -> bool {
    let midpoint = page_bounds.x + page_bounds.width * 0.5;
    let body_font_size = median_body_font_size(spans);
    let components = horizontal_span_components(spans, body_font_size);
    let mut wide_text_rows = 0usize;
    for component in components {
        let crosses_midpoint =
            component.bbox.x < midpoint && component.bbox.x + component.bbox.width > midpoint;
        let contiguous_across_midpoint = component.indices.iter().any(|index| {
            let bbox = spans[*index].bbox;
            bbox.x < midpoint && bbox.x + bbox.width > midpoint
        }) || {
            let left_edge = component
                .indices
                .iter()
                .map(|index| spans[*index].bbox)
                .filter(|bbox| bbox.x + bbox.width <= midpoint)
                .map(|bbox| bbox.x + bbox.width)
                .max_by(f32::total_cmp);
            let right_edge = component
                .indices
                .iter()
                .map(|index| spans[*index].bbox)
                .filter(|bbox| bbox.x >= midpoint)
                .map(|bbox| bbox.x)
                .min_by(f32::total_cmp);
            left_edge
                .zip(right_edge)
                .is_some_and(|(left, right)| right - left <= body_font_size)
        };
        if crosses_midpoint && contiguous_across_midpoint {
            if component
                .indices
                .iter()
                .any(|index| is_math_span(&spans[*index]))
            {
                return true;
            }
            if component.bbox.width >= page_bounds.width * 0.5 {
                wide_text_rows += 1;
            }
        }
    }
    if wide_text_rows >= 2 {
        return true;
    }

    graphic_components(graphic_bounds, page_bounds, body_font_size * 1.5)
        .into_iter()
        .any(|bbox| {
            bbox.x < midpoint
                && bbox.x + bbox.width > midpoint
                && bbox.width >= page_bounds.width * 0.45
                && bbox.height >= body_font_size * 2.0
        })
}

fn graphic_components(bounds: &[Rect], page_bounds: Rect, proximity: f32) -> Vec<Rect> {
    let cell_size = proximity.max(8.0);
    let columns = (page_bounds.width / cell_size).ceil().max(1.0) as usize;
    let rows = (page_bounds.height / cell_size).ceil().max(1.0) as usize;
    let mut occupied = vec![false; columns.saturating_mul(rows)];

    for &bound in bounds {
        let Some(expanded) = intersect_rect(expand_rect(bound, proximity), page_bounds) else {
            continue;
        };
        let first_column =
            (((expanded.x - page_bounds.x) / cell_size).floor().max(0.0) as usize).min(columns - 1);
        let last_column = (((expanded.x + expanded.width - page_bounds.x) / cell_size)
            .floor()
            .max(0.0) as usize)
            .min(columns - 1);
        let first_row =
            (((expanded.y - page_bounds.y) / cell_size).floor().max(0.0) as usize).min(rows - 1);
        let last_row = (((expanded.y + expanded.height - page_bounds.y) / cell_size)
            .floor()
            .max(0.0) as usize)
            .min(rows - 1);
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                occupied[row * columns + column] = true;
            }
        }
    }

    let mut visited = vec![false; occupied.len()];
    let mut components = Vec::new();
    for start in 0..occupied.len() {
        if !occupied[start] || visited[start] {
            continue;
        }
        let mut stack = vec![start];
        visited[start] = true;
        let mut minimum_column = start % columns;
        let mut maximum_column = minimum_column;
        let mut minimum_row = start / columns;
        let mut maximum_row = minimum_row;
        while let Some(cell) = stack.pop() {
            let column = cell % columns;
            let row = cell / columns;
            minimum_column = minimum_column.min(column);
            maximum_column = maximum_column.max(column);
            minimum_row = minimum_row.min(row);
            maximum_row = maximum_row.max(row);
            for (next_column, next_row) in [
                column.checked_sub(1).map(|value| (value, row)),
                (column + 1 < columns).then_some((column + 1, row)),
                row.checked_sub(1).map(|value| (column, value)),
                (row + 1 < rows).then_some((column, row + 1)),
            ]
            .into_iter()
            .flatten()
            {
                let next = next_row * columns + next_column;
                if occupied[next] && !visited[next] {
                    visited[next] = true;
                    stack.push(next);
                }
            }
        }
        let x = page_bounds.x + minimum_column as f32 * cell_size;
        let y = page_bounds.y + minimum_row as f32 * cell_size;
        let right = (page_bounds.x + (maximum_column + 1) as f32 * cell_size)
            .min(page_bounds.x + page_bounds.width);
        let top = (page_bounds.y + (maximum_row + 1) as f32 * cell_size)
            .min(page_bounds.y + page_bounds.height);
        components.push(Rect::new(x, y, right - x, top - y));
    }
    components
}

fn split_visual_column(column: Rect, spans: &[TextSpan], graphic_bounds: &[Rect]) -> Vec<Rect> {
    let maximum_height = (column.width * 1.45).max(240.0);
    if column.height <= maximum_height * 1.1 {
        return vec![column];
    }

    let column_bottom = column.y;
    let mut current_top = column.y + column.height;
    let mut regions = Vec::new();
    while current_top - column_bottom > maximum_height * 1.1 {
        let ideal_cut = current_top - maximum_height;
        let minimum_cut = column_bottom + 80.0;
        let maximum_cut = current_top - 80.0;
        let Some(cut) = nearest_clear_horizontal_cut(
            ideal_cut.clamp(minimum_cut, maximum_cut),
            minimum_cut,
            maximum_cut,
            column,
            spans,
            graphic_bounds,
        ) else {
            return vec![column];
        };
        regions.push(Rect::new(column.x, cut, column.width, current_top - cut));
        current_top = cut;
    }
    regions.push(Rect::new(
        column.x,
        column_bottom,
        column.width,
        current_top - column_bottom,
    ));
    regions
}

fn nearest_clear_horizontal_cut(
    ideal: f32,
    minimum: f32,
    maximum: f32,
    column: Rect,
    spans: &[TextSpan],
    graphic_bounds: &[Rect],
) -> Option<f32> {
    let search_radius = (maximum - minimum).ceil().max(0.0) as usize;
    for offset in 0..=search_radius {
        let distance = offset as f32;
        for candidate in [ideal + distance, ideal - distance] {
            if candidate < minimum || candidate > maximum {
                continue;
            }
            let crosses_text = spans.iter().any(|span| {
                materially_overlaps_horizontally(span.bbox, column)
                    && span.bbox.y - 1.0 < candidate
                    && span.bbox.y + span.bbox.height + 1.0 > candidate
            });
            let crosses_graphic = graphic_bounds.iter().any(|bbox| {
                materially_overlaps_horizontally(*bbox, column)
                    && bbox.y - 2.0 < candidate
                    && bbox.y + bbox.height + 2.0 > candidate
            });
            if !crosses_text && !crosses_graphic {
                return Some(candidate);
            }
        }
    }
    None
}

fn materially_overlaps_horizontally(content: Rect, region: Rect) -> bool {
    let overlap =
        (content.x + content.width).min(region.x + region.width) - content.x.max(region.x);
    overlap > 0.0 && overlap >= content.width.min(region.width) * 0.25
}

pub(super) fn horizontal_span_components(
    spans: &[TextSpan],
    body_font_size: f32,
) -> Vec<SpanComponent> {
    let mut ordered: Vec<usize> = (0..spans.len())
        .filter(|index| {
            let span = &spans[*index];
            !span.text.trim().is_empty()
                && span.rotation_degrees.abs() < 1.0
                && span.artifact_type.is_none()
        })
        .collect();
    ordered.sort_by(|left, right| {
        spans[*right]
            .bbox
            .y
            .total_cmp(&spans[*left].bbox.y)
            .then(spans[*left].bbox.x.total_cmp(&spans[*right].bbox.x))
    });

    let mut rows: Vec<(f32, Vec<usize>)> = Vec::new();
    for index in ordered {
        let center_y = spans[index].bbox.y + spans[index].bbox.height * 0.5;
        if let Some((_, indices)) = rows
            .iter_mut()
            .find(|(baseline, _)| (center_y - *baseline).abs() <= body_font_size * 0.4)
        {
            indices.push(index);
        } else {
            rows.push((center_y, vec![index]));
        }
    }

    let mut components = Vec::new();
    for (_, mut indices) in rows {
        indices.sort_by(|left, right| spans[*left].bbox.x.total_cmp(&spans[*right].bbox.x));
        let mut current = Vec::new();
        let mut right_edge = f32::NEG_INFINITY;
        for index in indices {
            let span = &spans[index];
            if !current.is_empty() && span.bbox.x - right_edge > body_font_size * 2.5 {
                components.push(component_from_indices(std::mem::take(&mut current), spans));
            }
            right_edge = right_edge.max(span.bbox.x + span.bbox.width);
            current.push(index);
        }
        if !current.is_empty() {
            components.push(component_from_indices(current, spans));
        }
    }
    components
}

fn component_from_indices(indices: Vec<usize>, spans: &[TextSpan]) -> SpanComponent {
    let bbox = indices
        .iter()
        .skip(1)
        .fold(spans[indices[0]].bbox, |bbox, index| {
            union_rect(bbox, spans[*index].bbox)
        });
    SpanComponent { indices, bbox }
}

pub(super) fn local_column_bounds(bbox: Rect, page_bounds: Rect, two_columns: bool) -> (f32, f32) {
    let page_right = page_bounds.x + page_bounds.width;
    if !two_columns {
        return (page_bounds.x, page_right);
    }
    let midpoint = page_bounds.x + page_bounds.width * 0.5;
    if bbox.x + bbox.width * 0.5 < midpoint {
        (page_bounds.x, midpoint)
    } else {
        (midpoint, page_right)
    }
}

pub(super) fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width
        && left.x + left.width > right.x
        && left.y < right.y + right.height
        && left.y + left.height > right.y
}

pub(super) fn rect_vertical_gap(left: Rect, right: Rect) -> f32 {
    let left_top = left.y + left.height;
    let right_top = right.y + right.height;
    if left_top < right.y {
        right.y - left_top
    } else if right_top < left.y {
        left.y - right_top
    } else {
        0.0
    }
}

pub(super) fn median_body_font_size(spans: &[TextSpan]) -> f32 {
    let mut sizes: Vec<f32> = spans
        .iter()
        .filter(|span| {
            span.heading_level.is_none()
                && span.font_size.is_finite()
                && (5.0..=24.0).contains(&span.font_size)
                && span.text.chars().any(char::is_alphabetic)
        })
        .map(|span| span.font_size)
        .collect();
    sizes.sort_by(f32::total_cmp);
    sizes.get(sizes.len() / 2).copied().unwrap_or(10.0)
}

pub(super) fn page_has_two_text_columns(spans: &[TextSpan], page_bounds: Rect) -> bool {
    let midpoint = page_bounds.x + page_bounds.width * 0.5;
    let minimum_width = page_bounds.width * 0.2;
    let mut left = 0usize;
    let mut right = 0usize;
    let mut crossing = 0usize;
    for span in spans.iter().filter(|span| {
        span.bbox.width >= minimum_width
            && span
                .text
                .chars()
                .filter(|character| character.is_alphabetic())
                .count()
                >= 20
    }) {
        let right_edge = span.bbox.x + span.bbox.width;
        if right_edge < midpoint - 5.0 {
            left += 1;
        } else if span.bbox.x > midpoint + 5.0 {
            right += 1;
        } else {
            crossing += 1;
        }
    }
    left >= 4 && right >= 4 && crossing <= 2
}

pub(super) fn overlaps_any(subject: Rect, regions: &[Rect]) -> bool {
    regions
        .iter()
        .any(|region| overlap_fraction(subject, *region) >= 0.25)
}

pub(super) fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let top_edge = (left.y + left.height).max(right.y + right.height);
    Rect::new(x, y, right_edge - x, top_edge - y)
}

pub(super) fn intersect_rect(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let top_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && top_edge > y).then(|| Rect::new(x, y, right_edge - x, top_edge - y))
}

pub(super) fn expand_rect(rect: Rect, amount: f32) -> Rect {
    Rect::new(
        rect.x - amount,
        rect.y - amount,
        rect.width + amount * 2.0,
        rect.height + amount * 2.0,
    )
}

pub(super) fn overlap_fraction(subject: Rect, region: Rect) -> f32 {
    let left = subject.x.max(region.x);
    let bottom = subject.y.max(region.y);
    let right = (subject.x + subject.width).min(region.x + region.width);
    let top = (subject.y + subject.height).min(region.y + region.height);
    let subject_area = subject.width.max(0.0) * subject.height.max(0.0);
    if right <= left || top <= bottom || subject_area <= 0.0 {
        0.0
    } else {
        ((right - left) * (top - bottom) / subject_area).clamp(0.0, 1.0)
    }
}
