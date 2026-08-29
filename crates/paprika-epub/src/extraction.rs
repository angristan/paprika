use pdf_oxide::PdfDocument;
use pdf_oxide::converters::ConversionOptions;
use std::collections::{HashMap, HashSet};

use super::EpubError;
use super::model::{ImagePlacement, PageImage, SemanticPage};
use super::sanitization::{
    escape_xml, find_equation_anchor_paragraph, markdown_to_xhtml,
    replace_caption_paragraph_with_figure, replace_equation_anchor_with_image,
    strip_invalid_xml_characters, strip_markup, xhtml_document,
};

#[derive(Default)]
pub(super) struct RepeatedRunningText {
    pub(super) headers: HashSet<String>,
    pub(super) footers: HashSet<String>,
}

pub(super) fn collect_repeated_running_text(
    document: &PdfDocument,
    page_count: usize,
) -> RepeatedRunningText {
    if page_count < 3 {
        return RepeatedRunningText::default();
    }
    let minimum_occurrences = ((page_count as f32 * 0.6).ceil() as usize).max(2);
    let mut header_occurrences = HashMap::<String, usize>::new();
    let mut footer_occurrences = HashMap::<String, usize>::new();
    for page_index in 0..page_count {
        let page_height = document
            .get_page_media_box(page_index)
            .map(|media| media.3)
            .unwrap_or(792.0);
        let Ok(spans) = document.extract_spans(page_index) else {
            continue;
        };
        let mut seen_headers = HashSet::new();
        let mut seen_footers = HashSet::new();
        for span in spans {
            let normalized = normalize_running_text(&span.text);
            if normalized.len() <= 3 {
                continue;
            }
            if span.bbox.y > page_height * 0.85 && seen_headers.insert(normalized.clone()) {
                *header_occurrences.entry(normalized.clone()).or_default() += 1;
            }
            if span.bbox.y + span.bbox.height < page_height * 0.15
                && seen_footers.insert(normalized.clone())
            {
                *footer_occurrences.entry(normalized).or_default() += 1;
            }
        }
    }
    let frequent = |occurrences: HashMap<String, usize>| {
        occurrences
            .into_iter()
            .filter_map(|(text, count)| (count >= minimum_occurrences).then_some(text))
            .collect()
    };
    RepeatedRunningText {
        headers: frequent(header_occurrences),
        footers: frequent(footer_occurrences),
    }
}

fn normalize_running_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_ascii_digit())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn strip_repeated_running_html(html: &mut String, repeated: &RepeatedRunningText) {
    if repeated.headers.is_empty() && repeated.footers.is_empty() {
        return;
    }
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
            let content_end = content_start + relative_close;
            let end = content_end + closing.len();
            let normalized =
                normalize_running_text(&strip_markup(&html[content_start..content_end]));
            blocks.push((start, end, normalized));
            search_from = end;
        }
    }
    blocks.sort_unstable_by_key(|block| block.0);
    let mut removals = Vec::new();
    let mut removed_headers = HashSet::new();
    for (index, (start, end, normalized)) in blocks.iter().enumerate() {
        if index < 2
            && repeated.headers.contains(normalized)
            && removed_headers.insert(normalized.clone())
        {
            removals.push((*start, *end));
        }
    }
    let mut removed_footers = HashSet::new();
    for (index, (start, end, normalized)) in blocks.iter().enumerate().rev() {
        if index.saturating_add(2) >= blocks.len()
            && repeated.footers.contains(normalized)
            && removed_footers.insert(normalized.clone())
        {
            removals.push((*start, *end));
        }
    }
    removals.sort_unstable();
    removals.dedup();
    for (start, end) in removals.into_iter().rev() {
        html.replace_range(start..end, "");
    }
}

pub(super) fn extract_page_xhtml(
    document: &PdfDocument,
    page_index: usize,
    options: &ConversionOptions,
    repeated_running_text: &RepeatedRunningText,
) -> Result<(String, String), EpubError> {
    let markdown =
        document
            .to_markdown(page_index, options)
            .map_err(|error| EpubError::Extract {
                page: page_index + 1,
                message: error.to_string(),
            })?;
    let mut html = markdown_to_xhtml(&markdown);
    strip_repeated_running_html(&mut html, repeated_running_text);
    strip_invalid_xml_characters(&mut html);
    Ok((markdown, html))
}

pub(super) fn equation_anchors_are_unique(html: &str, images: &[PageImage]) -> bool {
    let mut matched_paragraphs = Vec::new();
    for image in images {
        let ImagePlacement::EquationAnchor(anchor) = &image.placement else {
            continue;
        };
        let Some((start, end, _, _)) = find_equation_anchor_paragraph(html, anchor) else {
            return false;
        };
        if matched_paragraphs.contains(&(start, end)) {
            return false;
        }
        matched_paragraphs.push((start, end));
    }
    true
}

pub(super) fn render_semantic_page_body(page: &SemanticPage) -> Result<String, EpubError> {
    let mut body = format!(
        "<main class=\"source-page-content\" data-source-page=\"{}\">\n<p id=\"page-{}\" class=\"source-page\" epub:type=\"pagebreak\" role=\"doc-pagebreak\" aria-label=\"{}\">Source page {}</p>\n{}",
        page.number, page.number, page.number, page.number, page.html
    );
    let mut deferred_images = String::new();
    for image in &page.images {
        let source = format!("../{}", escape_xml(&image.href));
        let alt = escape_xml(&image.alt);
        let visual_page_fallback = matches!(&image.placement, ImagePlacement::VisualPageFallback);
        let placed = match &image.placement {
            ImagePlacement::Caption(marker) => {
                replace_caption_paragraph_with_figure(&mut body, marker, &source, &alt)
            }
            ImagePlacement::EquationAnchor(anchor) => {
                replace_equation_anchor_with_image(&mut body, anchor, &source, &alt)
            }
            ImagePlacement::VisualPageFallback | ImagePlacement::EndOfPage => false,
        };
        if !placed {
            if matches!(&image.placement, ImagePlacement::EquationAnchor(_)) {
                // Equation text was removed only after this anchor was
                // validated. Fail closed if packaging sees a different
                // chapter instead of silently dropping mathematical content.
                return Err(EpubError::Package(format!(
                    "validated equation anchor disappeared from source page {}",
                    page.number
                )));
            }
            if visual_page_fallback {
                deferred_images.push_str(&format!(
                    "<figure class=\"visual-page-fallback\"><img src=\"{source}\" alt=\"{alt}\"/></figure>\n"
                ));
            } else {
                deferred_images.push_str(&format!(
                    "<figure class=\"figure-fallback\"><img src=\"{source}\" alt=\"{alt}\"/><figcaption>{alt}</figcaption></figure>\n"
                ));
            }
        }
    }
    if !deferred_images.is_empty() {
        body.push_str(
            "<section class=\"page-images\" aria-label=\"Images from this source page\">\n",
        );
        body.push_str(&deferred_images);
        body.push_str("</section>\n");
    }
    body.push_str("</main>\n");
    Ok(body)
}

pub(super) fn account_rendered_xhtml(
    page: &SemanticPage,
    language: &str,
    total_bytes: &mut usize,
    max_bytes: usize,
) -> Result<(), EpubError> {
    let body = render_semantic_page_body(page)?;
    let xhtml = xhtml_document(&page.title, language, &body);
    *total_bytes = total_bytes
        .checked_add(xhtml.len())
        .ok_or(EpubError::SemanticTooLarge {
            limit: max_bytes / (1024 * 1024),
        })?;
    if *total_bytes > max_bytes {
        return Err(EpubError::SemanticTooLarge {
            limit: max_bytes / (1024 * 1024),
        });
    }
    Ok(())
}
