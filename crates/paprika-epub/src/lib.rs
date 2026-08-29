//! Semantic PDF-to-EPUB conversion shared by the native and browser clients.
//!
//! The converter keeps born-digital text selectable and lets the EPUB reader
//! perform pagination. It also carries useful raster image XObjects across as
//! EPUB resources. Pages without a usable text layer are called out explicitly
//! instead of silently producing an empty book.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as SourcePdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use pdf_oxide::PdfDocument;
use pdf_oxide::converters::{ConversionOptions, ReadingOrderMode};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::{RectFilterMode, TextSpan};
use pulldown_cmark::{CowStr, Event, Options as MarkdownOptions, Parser, Tag, TagEnd, html};
use rbook::Epub;
use rbook::epub::EpubChapter;
use std::io::{Cursor, Seek, SeekFrom, Write};
use thiserror::Error;

const DEFAULT_LANGUAGE: &str = "en";
const MAX_IMAGE_PIXELS: u64 = 4_000_000;
const MIN_IMAGE_PIXELS: u64 = 4_096;
const DEFAULT_MAX_ASSET_BYTES: usize = 96 * 1024 * 1024;
const DEFAULT_MAX_SEMANTIC_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_FIGURE_RENDER_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FIGURE_RENDER_PIXELS: u64 = 8_000_000;
const MAX_FIGURE_RENDER_EDGE: u32 = 8_192;
const FIGURE_RENDER_SCALE: f32 = 2.0;
const FIGURE_CROP_HEIGHT_POINTS: f32 = 225.0;
const FORMULA_HEAVY_MIN_MATH_SPANS: usize = 100;
const FORMULA_HEAVY_MIN_MATH_CHARACTERS: usize = 100;

const EPUB_CSS: &str = r#"
:root { color-scheme: light dark; }
body {
  font-family: serif;
  line-height: 1.5;
  margin: 5%;
  orphans: 2;
  widows: 2;
}
h1, h2, h3, h4, h5, h6 {
  break-after: avoid;
  font-family: sans-serif;
  line-height: 1.2;
}
p { margin: 0 0 0.8em; }
a { overflow-wrap: anywhere; }
table {
  border-collapse: collapse;
  display: block;
  font-size: 0.82em;
  max-width: 100%;
  overflow-x: auto;
}
th, td { border: 1px solid currentColor; padding: 0.25em; }
figure { break-inside: avoid; margin: 1em 0; text-align: center; }
figure img { height: auto; max-width: 100%; }
.visual-page-fallback { margin: 0; }
.visual-page-fallback img { display: block; width: 100%; }
figcaption, .source-page, .conversion-note {
  color: #666;
  font-family: sans-serif;
  font-size: 0.75em;
}
.source-page { border-bottom: 1px solid #aaa; padding-bottom: 0.35em; }
.conversion-note { border-left: 0.25em solid #c65d2e; padding-left: 0.7em; }
pre, code { font-family: monospace; white-space: pre-wrap; }
.algorithm {
  background: rgba(127, 127, 127, 0.09);
  border: 1px solid #aaa;
  font-size: 0.78em;
  overflow-wrap: anywhere;
  padding: 0.75em;
}
"#;

/// Controls semantic extraction and EPUB resource limits.
#[derive(Clone, Debug)]
pub struct EpubOptions {
    /// Reader-visible book title. Callers normally derive this from the file name.
    pub title: String,
    /// BCP 47 language tag used in EPUB metadata and XHTML.
    pub language: String,
    /// Preserve useful embedded raster images as EPUB resources.
    pub include_images: bool,
    /// Maximum number of source pages accepted by the semantic converter.
    pub max_pages: usize,
    /// Maximum cumulative uncompressed image-resource bytes.
    pub max_asset_bytes: usize,
    /// Maximum cumulative XHTML bytes before EPUB packaging.
    pub max_semantic_bytes: usize,
    /// Maximum final EPUB archive size.
    pub max_output_bytes: usize,
}

impl Default for EpubOptions {
    fn default() -> Self {
        Self {
            title: "Converted document".to_string(),
            language: DEFAULT_LANGUAGE.to_string(),
            include_images: true,
            max_pages: 10_000,
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            max_semantic_bytes: DEFAULT_MAX_SEMANTIC_BYTES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

/// In-memory EPUB plus a concise conversion report for user interfaces.
#[derive(Debug)]
pub struct ConvertedEpub {
    pub bytes: Vec<u8>,
    pub source_pages: usize,
    pub text_pages: usize,
    pub image_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum EpubError {
    #[error("could not parse the input PDF (encrypted PDFs are not supported): {0}")]
    Parse(String),
    #[error("the PDF contains no pages")]
    Empty,
    #[error("the PDF contains {pages} pages; the configured limit is {limit}")]
    TooManyPages { pages: usize, limit: usize },
    #[error("could not extract source page {page}: {message}")]
    Extract { page: usize, message: String },
    #[error("could not package EPUB: {0}")]
    Package(String),
    #[error("embedded images exceed the {limit} MiB memory limit")]
    AssetsTooLarge { limit: usize },
    #[error("semantic output exceeds the {limit} MiB memory limit")]
    SemanticTooLarge { limit: usize },
    #[error("EPUB output exceeds the {limit} MiB memory limit")]
    OutputTooLarge { limit: usize },
}

#[derive(Debug)]
struct SemanticPage {
    number: usize,
    title: String,
    html: String,
    images: Vec<PageImage>,
    has_text: bool,
}

#[derive(Debug)]
struct PageImage {
    href: String,
    bytes: Vec<u8>,
    alt: String,
    placement: ImagePlacement,
}

#[derive(Debug)]
enum ImagePlacement {
    Caption(String),
    EquationAnchor(String),
    VisualPageFallback,
    EndOfPage,
}

#[derive(Debug, Default)]
struct FigureCrops {
    images: Vec<PageImage>,
    regions: Vec<Rect>,
    text_exclusions: Vec<Rect>,
}

#[derive(Debug, Default)]
struct EquationCrops {
    images: Vec<PageImage>,
    text_exclusions: Vec<Rect>,
    asset_bytes: usize,
}

#[derive(Clone, Debug)]
struct EquationPlan {
    render_bbox: Rect,
    exclusion_rects: Vec<Rect>,
    anchor: String,
}

#[derive(Clone, Debug)]
struct SpanComponent {
    indices: Vec<usize>,
    bbox: Rect,
}

struct BoundedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: usize,
    limit_exceeded: bool,
}

impl BoundedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
            limit_exceeded: false,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let end = self.inner.position().saturating_add(buffer.len() as u64);
        if end > self.limit as u64 {
            self.limit_exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "encoded image exceeds its memory budget",
            ));
        }
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for BoundedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let next = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => add_signed_offset(self.inner.get_ref().len() as u64, offset),
            SeekFrom::Current(offset) => add_signed_offset(self.inner.position(), offset),
        };
        let Some(next) = next else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek before start of image buffer",
            ));
        };
        if next > self.limit as u64 {
            self.limit_exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "encoded image exceeds its memory budget",
            ));
        }
        self.inner.seek(SeekFrom::Start(next))
    }
}

fn add_signed_offset(base: u64, offset: i64) -> Option<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    }
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

/// Convert PDF bytes to a reflowable EPUB 3 archive entirely in memory.
pub fn convert_pdf_to_epub(input: &[u8], options: EpubOptions) -> Result<ConvertedEpub, EpubError> {
    let document = PdfDocument::from_bytes(input.to_vec())
        .map_err(|error| EpubError::Parse(error.to_string()))?;
    let page_count = document
        .page_count()
        .map_err(|error| EpubError::Parse(error.to_string()))?;
    if page_count == 0 {
        return Err(EpubError::Empty);
    }
    if page_count > options.max_pages {
        return Err(EpubError::TooManyPages {
            pages: page_count,
            limit: options.max_pages,
        });
    }

    let extraction_options = ConversionOptions {
        preserve_layout: false,
        detect_headings: true,
        extract_tables: true,
        include_images: false,
        strip_running_headers_footers: true,
        reading_order_mode: ReadingOrderMode::ColumnAware,
        expand_ligatures: true,
        include_artifacts: false,
        ..Default::default()
    };

    let mut semantic_pages = Vec::with_capacity(page_count);
    let mut warnings = Vec::new();
    let render_document = if options.include_images && input.len() <= MAX_FIGURE_RENDER_INPUT_BYTES
    {
        SourcePdf::new(input.to_vec()).ok()
    } else {
        if options.include_images && input.len() > MAX_FIGURE_RENDER_INPUT_BYTES {
            warnings.push(
                "Figure and equation crops were skipped because the source exceeds 32 MiB."
                    .to_string(),
            );
        }
        None
    };
    let mut asset_bytes = 0usize;
    let mut semantic_bytes = 0usize;
    let mut image_count = 0usize;
    let mut text_pages = 0usize;

    for page_index in 0..page_count {
        let page_asset_bytes = asset_bytes;
        let crops = if options.include_images {
            render_document
                .as_ref()
                .map(|source| {
                    collect_figure_crops(
                        source,
                        &document,
                        page_index,
                        &mut asset_bytes,
                        options.max_asset_bytes,
                        &mut warnings,
                    )
                })
                .transpose()?
                .unwrap_or_default()
        } else {
            FigureCrops::default()
        };
        if let Some(source) = render_document.as_ref().filter(|_| options.include_images) {
            let spans = document.extract_spans(page_index).unwrap_or_default();
            if is_math_dense_candidate(&spans, &crops.regions) {
                let mut probe_options = extraction_options.clone();
                probe_options.exclude_regions = crops.text_exclusions.clone();
                probe_options.exclude_regions_mode = RectFilterMode::MinOverlap(0.5);
                let (_, probe_html) = extract_page_xhtml(&document, page_index, &probe_options)?;
                let should_fallback =
                    math_extraction_is_unreliable(&spans, &crops.regions, &probe_html);
                if should_fallback {
                    let mut fallback_asset_bytes = page_asset_bytes;
                    let images = collect_formula_page_crops(
                        source,
                        &document,
                        page_index,
                        &spans,
                        &mut fallback_asset_bytes,
                        options.max_asset_bytes,
                        &mut warnings,
                    )?;
                    if !images.is_empty() {
                        let html = format!(
                            "<p class=\"conversion-note\">Source page {} contains dense mathematical layout that could not be represented reliably as selectable text. It is preserved visually in reading order.</p>\n",
                            page_index + 1
                        );
                        semantic_bytes = semantic_bytes.checked_add(html.len()).ok_or(
                            EpubError::SemanticTooLarge {
                                limit: options.max_semantic_bytes / (1024 * 1024),
                            },
                        )?;
                        if semantic_bytes > options.max_semantic_bytes {
                            return Err(EpubError::SemanticTooLarge {
                                limit: options.max_semantic_bytes / (1024 * 1024),
                            });
                        }
                        asset_bytes = fallback_asset_bytes;
                        image_count += images.len();
                        warnings.push(format!(
                        "Page {} has dense mathematical layout and was preserved as visual columns.",
                        page_index + 1
                    ));
                        semantic_pages.push(SemanticPage {
                            number: page_index + 1,
                            title: format!("Page {}", page_index + 1),
                            html,
                            images,
                            has_text: false,
                        });
                        continue;
                    }
                }
            }
        }
        let mut equations = if options.include_images {
            render_document
                .as_ref()
                .map(|source| {
                    collect_equation_crops(
                        source,
                        &document,
                        page_index,
                        &crops.regions,
                        asset_bytes,
                        options.max_asset_bytes,
                        &mut warnings,
                    )
                })
                .transpose()?
                .unwrap_or_default()
        } else {
            EquationCrops::default()
        };
        let mut page_extraction_options = extraction_options.clone();
        page_extraction_options.exclude_regions = crops.text_exclusions.clone();
        page_extraction_options
            .exclude_regions
            .extend(equations.text_exclusions.iter().copied());
        page_extraction_options.exclude_regions_mode = RectFilterMode::MinOverlap(0.5);
        let (mut markdown, mut html) =
            extract_page_xhtml(&document, page_index, &page_extraction_options)?;
        if !equation_anchors_are_unique(&html, &equations.images) {
            warnings.push(format!(
                "Display equations from page {} remained as text because their reading position was ambiguous.",
                page_index + 1
            ));
            equations = EquationCrops::default();
            page_extraction_options.exclude_regions = crops.text_exclusions.clone();
            (markdown, html) = extract_page_xhtml(&document, page_index, &page_extraction_options)?;
        }
        let remaining_semantic_bytes = options.max_semantic_bytes.saturating_sub(semantic_bytes);
        if markdown.len() > remaining_semantic_bytes {
            return Err(EpubError::SemanticTooLarge {
                limit: options.max_semantic_bytes / (1024 * 1024),
            });
        }
        asset_bytes =
            asset_bytes
                .checked_add(equations.asset_bytes)
                .ok_or(EpubError::AssetsTooLarge {
                    limit: options.max_asset_bytes / (1024 * 1024),
                })?;
        let has_text = visible_text_len(&html) >= 20;
        if has_text {
            text_pages += 1;
        } else {
            warnings.push(format!(
                "Page {} has no reliable text layer; OCR was not attempted.",
                page_index + 1
            ));
            html.push_str(
                "<p class=\"conversion-note\">This source page has no reliable text layer. Paprika did not run OCR.</p>\n",
            );
        }

        enhance_algorithm_blocks(&mut html);
        semantic_bytes =
            semantic_bytes
                .checked_add(html.len())
                .ok_or(EpubError::SemanticTooLarge {
                    limit: options.max_semantic_bytes / (1024 * 1024),
                })?;
        if semantic_bytes > options.max_semantic_bytes {
            return Err(EpubError::SemanticTooLarge {
                limit: options.max_semantic_bytes / (1024 * 1024),
            });
        }
        let images = if options.include_images {
            let FigureCrops {
                mut images,
                regions,
                ..
            } = crops;
            images.extend(equations.images);
            images.extend(collect_page_images(
                &document,
                page_index,
                &regions,
                &mut asset_bytes,
                options.max_asset_bytes,
                &mut warnings,
            )?);
            images
        } else {
            Vec::new()
        };
        image_count += images.len();
        let title = first_heading(&html).unwrap_or_else(|| format!("Page {}", page_index + 1));
        semantic_pages.push(SemanticPage {
            number: page_index + 1,
            title,
            html,
            images,
            has_text,
        });
    }

    if text_pages == 0 {
        warnings.push(
            "No selectable text was found. This EPUB contains only recoverable embedded images; use OCR for semantic output."
                .to_string(),
        );
    }

    let title = normalized_title(&options.title);
    let language = normalized_language(&options.language);
    let identifier = document_identifier(input);
    let bytes = package_epub(&title, &language, &identifier, semantic_pages)?;
    if bytes.len() > options.max_output_bytes {
        return Err(EpubError::OutputTooLarge {
            limit: options.max_output_bytes / (1024 * 1024),
        });
    }

    Ok(ConvertedEpub {
        bytes,
        source_pages: page_count,
        text_pages,
        image_count,
        warnings,
    })
}

fn is_math_dense_candidate(spans: &[TextSpan], excluded_regions: &[Rect]) -> bool {
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

fn math_extraction_is_unreliable(
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

fn visual_page_regions(
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

fn collect_formula_page_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    page_index: usize,
    spans: &[TextSpan],
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<PageImage>, EpubError> {
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
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
    let image_bounds: Vec<Rect> = document
        .page_image_handles(page_index)
        .map(|images| images.into_iter().map(|image| image.bbox).collect())
        .unwrap_or_default();
    graphic_bounds.extend(image_bounds);
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
        if let Err(error) =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut output, image::ImageFormat::Png)
        {
            if output.limit_exceeded {
                return Err(EpubError::AssetsTooLarge {
                    limit: max_bytes / (1024 * 1024),
                });
            }
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

fn collect_figure_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    page_index: usize,
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<FigureCrops, EpubError> {
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
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
    let image_bounds: Vec<Rect> = document
        .page_image_handles(page_index)
        .map(|handles| handles.into_iter().map(|image| image.bbox).collect())
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
        let semantic_region =
            tighten_regions_from_graphics(&[coarse_region], &path_bounds, &image_bounds)
                .into_iter()
                .next();
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
        if let Err(error) =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut output, image::ImageFormat::Png)
        {
            if output.limit_exceeded {
                return Err(EpubError::AssetsTooLarge {
                    limit: max_bytes / (1024 * 1024),
                });
            }
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
        if let Some(region) = semantic_region {
            text_exclusions.push(region);
        }
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

fn collect_equation_crops(
    source: &SourcePdf,
    document: &PdfDocument,
    page_index: usize,
    excluded_regions: &[Rect],
    existing_asset_bytes: usize,
    max_asset_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<EquationCrops, EpubError> {
    if document.get_page_rotation(page_index).unwrap_or(0) != 0 {
        return Ok(EquationCrops::default());
    }
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

    let spans = match document.extract_spans(page_index) {
        Ok(spans) => spans,
        Err(error) => {
            warnings.push(format!(
                "Could not inspect equations from page {}: {error}",
                page_index + 1
            ));
            return Ok(EquationCrops::default());
        }
    };
    let table_regions: Vec<Rect> = document
        .extract_tables(page_index)
        .map(|tables| tables.into_iter().filter_map(|table| table.bbox).collect())
        .unwrap_or_default();
    let image_regions: Vec<Rect> = document
        .page_image_handles(page_index)
        .map(|images| images.into_iter().map(|image| image.bbox).collect())
        .unwrap_or_default();
    let mut veto_regions =
        Vec::with_capacity(excluded_regions.len() + table_regions.len() + image_regions.len());
    veto_regions.extend_from_slice(excluded_regions);
    veto_regions.extend(table_regions);
    veto_regions.extend(image_regions);
    let plans = find_display_equations(&spans, page_bounds, &veto_regions);
    if plans.is_empty() {
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
    let mut cumulative_asset_bytes = existing_asset_bytes;
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
        let remaining_bytes = max_asset_bytes.saturating_sub(cumulative_asset_bytes);
        let mut output = BoundedBuffer::new(remaining_bytes);
        if let Err(error) =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut output, image::ImageFormat::Png)
        {
            if output.limit_exceeded {
                return Err(EpubError::AssetsTooLarge {
                    limit: max_asset_bytes / (1024 * 1024),
                });
            }
            warnings.push(format!(
                "Could not encode equation {} from page {}: {error}",
                index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = output.into_inner();
        account_asset(&mut cumulative_asset_bytes, bytes.len(), max_asset_bytes)?;
        equation_asset_bytes =
            equation_asset_bytes
                .checked_add(bytes.len())
                .ok_or(EpubError::AssetsTooLarge {
                    limit: max_asset_bytes / (1024 * 1024),
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

fn find_display_equations(
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

fn horizontal_span_components(spans: &[TextSpan], body_font_size: f32) -> Vec<SpanComponent> {
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

fn local_column_bounds(bbox: Rect, page_bounds: Rect, two_columns: bool) -> (f32, f32) {
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

fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width
        && left.x + left.width > right.x
        && left.y < right.y + right.height
        && left.y + left.height > right.y
}

fn rect_vertical_gap(left: Rect, right: Rect) -> f32 {
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

fn median_body_font_size(spans: &[TextSpan]) -> f32 {
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

fn page_has_two_text_columns(spans: &[TextSpan], page_bounds: Rect) -> bool {
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

fn is_math_span(span: &TextSpan) -> bool {
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

fn overlaps_any(subject: Rect, regions: &[Rect]) -> bool {
    regions
        .iter()
        .any(|region| overlap_fraction(subject, *region) >= 0.25)
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

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let top_edge = (left.y + left.height).max(right.y + right.height);
    Rect::new(x, y, right_edge - x, top_edge - y)
}

fn tighten_regions_from_graphics(
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

fn intersect_rect(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let top_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && top_edge > y).then(|| Rect::new(x, y, right_edge - x, top_edge - y))
}

fn expand_rect(rect: Rect, amount: f32) -> Rect {
    Rect::new(
        rect.x - amount,
        rect.y - amount,
        rect.width + amount * 2.0,
        rect.height + amount * 2.0,
    )
}

fn account_asset(total_bytes: &mut usize, bytes: usize, max_bytes: usize) -> Result<(), EpubError> {
    *total_bytes = total_bytes
        .checked_add(bytes)
        .ok_or(EpubError::AssetsTooLarge {
            limit: max_bytes / (1024 * 1024),
        })?;
    if *total_bytes > max_bytes {
        return Err(EpubError::AssetsTooLarge {
            limit: max_bytes / (1024 * 1024),
        });
    }
    Ok(())
}

fn collect_page_images(
    document: &PdfDocument,
    page_index: usize,
    excluded_regions: &[Rect],
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<PageImage>, EpubError> {
    let handles = document
        .page_image_handles(page_index)
        .unwrap_or_else(|error| {
            warnings.push(format!(
                "Could not inspect images from page {}: {error}",
                page_index + 1
            ));
            Vec::new()
        });
    let mut images = Vec::new();

    for (image_index, handle) in handles.into_iter().enumerate() {
        let pixels = u64::from(handle.width) * u64::from(handle.height);
        let covered_by_figure = excluded_regions
            .iter()
            .any(|region| overlap_fraction(handle.bbox, *region) >= 0.5);
        if !(MIN_IMAGE_PIXELS..=MAX_IMAGE_PIXELS).contains(&pixels)
            || handle.byte_size_compressed > max_bytes as u64
            || covered_by_figure
        {
            continue;
        }
        let image = match handle.decode() {
            Ok(image) => image,
            Err(error) => {
                warnings.push(format!(
                    "Could not decode image {} from page {}: {error}",
                    image_index + 1,
                    page_index + 1
                ));
                continue;
            }
        };
        let bytes = match image.to_png_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                warnings.push(format!(
                    "Could not encode image {} from page {}: {error}",
                    image_index + 1,
                    page_index + 1
                ));
                continue;
            }
        };
        account_asset(total_bytes, bytes.len(), max_bytes)?;
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-{:02}.png",
                page_index + 1,
                image_index + 1
            ),
            bytes,
            alt: format!(
                "Image {} from source page {}",
                image_index + 1,
                page_index + 1
            ),
            placement: ImagePlacement::EndOfPage,
        });
    }

    Ok(images)
}

fn overlap_fraction(subject: Rect, region: Rect) -> f32 {
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

fn extract_page_xhtml(
    document: &PdfDocument,
    page_index: usize,
    options: &ConversionOptions,
) -> Result<(String, String), EpubError> {
    let markdown =
        document
            .to_markdown(page_index, options)
            .map_err(|error| EpubError::Extract {
                page: page_index + 1,
                message: error.to_string(),
            })?;
    let mut html = markdown_to_xhtml(&markdown);
    strip_invalid_xml_characters(&mut html);
    Ok((markdown, html))
}

fn equation_anchors_are_unique(html: &str, images: &[PageImage]) -> bool {
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

fn package_epub(
    title: &str,
    language: &str,
    identifier: &str,
    pages: Vec<SemanticPage>,
) -> Result<Vec<u8>, EpubError> {
    let mut editor = Epub::builder()
        .identifier(identifier)
        .title(title)
        .language(language)
        // EPUB 3 requires dcterms:modified. Fixed dates keep native and
        // browser output reproducible because wasm32 has no system clock.
        .published_date("2025-01-01")
        .modified_date("2025-01-01T00:00:00Z")
        .resource(("styles.css", EPUB_CSS));

    for page in pages {
        let chapter_href = format!("text/page-{:04}.xhtml", page.number);
        let mut body = format!(
            "<main class=\"source-page-content\" data-source-page=\"{}\">\n<p class=\"source-page\">Source page {}</p>\n{}",
            page.number, page.number, page.html
        );
        let mut deferred_images = String::new();
        for image in page.images {
            let source = format!("../{}", escape_xml(&image.href));
            let alt = escape_xml(&image.alt);
            let visual_page_fallback =
                matches!(&image.placement, ImagePlacement::VisualPageFallback);
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
                if matches!(image.placement, ImagePlacement::EquationAnchor(_)) {
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
            editor = editor.resource((image.href, image.bytes));
        }
        if !deferred_images.is_empty() {
            body.push_str(
                "<section class=\"page-images\" aria-label=\"Images from this source page\">\n",
            );
            body.push_str(&deferred_images);
            body.push_str("</section>\n");
        }
        body.push_str("</main>\n");

        let xhtml = xhtml_document(title, language, &body);
        let chapter_title = if page.has_text {
            page.title
        } else {
            format!("Page {} (image only)", page.number)
        };
        editor = editor.chapter(
            EpubChapter::new(chapter_title)
                .href(chapter_href)
                .xhtml(xhtml.into_bytes()),
        );
    }

    editor
        .write()
        .compression(9)
        .toc_stylesheet("styles.css")
        .to_vec()
        .map_err(|error| EpubError::Package(error.to_string()))
}

fn replace_caption_paragraph_with_figure(
    body: &mut String,
    caption_marker: &str,
    image_source: &str,
    image_alt: &str,
) -> bool {
    let mut search_from = 0usize;
    while let Some(relative_start) = body[search_from..].find("<p") {
        let paragraph_start = search_from + relative_start;
        let Some(relative_open_end) = body[paragraph_start..].find('>') else {
            return false;
        };
        let content_start = paragraph_start + relative_open_end + 1;
        let Some(relative_close) = body[content_start..].find("</p>") else {
            return false;
        };
        let content_end = content_start + relative_close;
        let paragraph_end = content_end + "</p>".len();
        let caption_markup = &body[content_start..content_end];
        let caption_text = strip_markup(caption_markup);
        if caption_text.trim_start().starts_with(caption_marker) {
            let figure = format!(
                "<figure class=\"figure-fallback\"><img src=\"{image_source}\" alt=\"{image_alt}\"/><figcaption>{caption_markup}</figcaption></figure>"
            );
            body.replace_range(paragraph_start..paragraph_end, &figure);
            return true;
        }
        search_from = paragraph_end;
    }
    false
}

fn replace_equation_anchor_with_image(
    body: &mut String,
    anchor: &str,
    image_source: &str,
    image_alt: &str,
) -> bool {
    let Some((paragraph_start, paragraph_end, content_start, anchor_start)) =
        find_equation_anchor_paragraph(body, anchor)
    else {
        return false;
    };
    let content_end = paragraph_end - "</p>".len();
    let content = &body[content_start..content_end];
    let prefix = content[..anchor_start].trim_end();
    let figure = format!(
        "<figure class=\"equation-fallback\"><img src=\"{image_source}\" alt=\"{image_alt}\"/></figure>"
    );
    let replacement = if strip_markup(prefix).trim().is_empty() {
        figure
    } else {
        format!(
            "{}{}</p>\n{figure}",
            &body[paragraph_start..content_start],
            prefix
        )
    };
    body.replace_range(paragraph_start..paragraph_end, &replacement);
    true
}

fn find_equation_anchor_paragraph(
    body: &str,
    anchor: &str,
) -> Option<(usize, usize, usize, usize)> {
    let mut search_from = 0usize;
    let mut match_result = None;
    while let Some(relative_start) = body[search_from..].find("<p") {
        let paragraph_start = search_from + relative_start;
        let relative_open_end = body[paragraph_start..].find('>')?;
        let content_start = paragraph_start + relative_open_end + 1;
        let relative_close = body[content_start..].find("</p>")?;
        let content_end = content_start + relative_close;
        let paragraph_end = content_end + "</p>".len();
        let content = &body[content_start..content_end];
        let text = strip_markup(content);
        if text.trim_end().ends_with(anchor) {
            let anchor_start = content.rfind(anchor)?;
            if !content[anchor_start + anchor.len()..].trim().is_empty() {
                return None;
            }
            if match_result.is_some() {
                return None;
            }
            match_result = Some((paragraph_start, paragraph_end, content_start, anchor_start));
        }
        search_from = paragraph_end;
    }
    match_result
}

fn enhance_algorithm_blocks(html: &mut String) {
    const START: &str = "<p><strong>Algorithm</strong>";
    const PARAGRAPH_END: &str = "</p>";
    let mut search_from = 0usize;
    while let Some(relative_start) = html[search_from..].find(START) {
        let start = search_from + relative_start;
        let Some(relative_end) = html[start..].find(PARAGRAPH_END) else {
            break;
        };
        let first_end = start + relative_end + PARAGRAPH_END.len();
        let mut consume_end = first_end;
        let mut continuation = String::new();

        // PDF generators often split one pseudocode box into several Markdown
        // paragraphs. Keep immediately following numbered paragraphs in the
        // same preformatted block, but stop before ordinary prose resumes.
        loop {
            let remainder = &html[consume_end..];
            let whitespace = remainder.len() - remainder.trim_start().len();
            let paragraph_start = consume_end + whitespace;
            if !html[paragraph_start..].starts_with("<p>") {
                break;
            }
            let Some(next_relative_end) = html[paragraph_start..].find(PARAGRAPH_END) else {
                break;
            };
            let paragraph_end = paragraph_start + next_relative_end + PARAGRAPH_END.len();
            let paragraph =
                &html[paragraph_start + "<p>".len()..paragraph_end - PARAGRAPH_END.len()];
            if !contains_algorithm_line_number(paragraph) {
                break;
            }
            continuation.push('\n');
            continuation.push_str(paragraph);
            consume_end = paragraph_end;
        }

        let first = &html[start + "<p>".len()..first_end - PARAGRAPH_END.len()];
        let mut block = format!("<pre class=\"algorithm\">{first}{continuation}</pre>");
        let title_number_end = block
            .find(":</strong>")
            .map_or(START.len(), |position| position + ":</strong>".len());
        let (heading, code) = block.split_at(title_number_end);
        let mut code = code.to_string();
        for line_number in (1..=99).rev() {
            for suffix in ["</strong> ", " "] {
                let marker = format!(" <strong>{line_number}{suffix}");
                let replacement = format!("\n<strong>{line_number}{suffix}");
                code = code.replace(&marker, &replacement);
            }
        }
        block = format!("{heading}{code}").replace("**", "");
        html.replace_range(start..consume_end, &block);
        search_from = start + block.len();
    }
}

fn contains_algorithm_line_number(paragraph: &str) -> bool {
    (1..=99).any(|line_number| {
        paragraph.contains(&format!("<strong>{line_number}</strong>"))
            || paragraph.contains(&format!("<strong>{line_number} "))
    })
}

fn markdown_to_xhtml(markdown: &str) -> String {
    let options = MarkdownOptions::ENABLE_TABLES
        | MarkdownOptions::ENABLE_FOOTNOTES
        | MarkdownOptions::ENABLE_STRIKETHROUGH
        | MarkdownOptions::ENABLE_TASKLISTS
        | MarkdownOptions::ENABLE_SMART_PUNCTUATION;
    // Treat raw HTML from a PDF as ordinary text. This keeps document content
    // from injecting active markup into the EPUB reader.
    let events = Parser::new_ext(markdown, options).filter_map(|event| match event {
        Event::Html(value) | Event::InlineHtml(value) => Some(Event::Text(CowStr::Boxed(
            value.into_string().into_boxed_str(),
        ))),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Some(Event::Start(Tag::Link {
            link_type,
            dest_url: safe_link_destination(dest_url),
            title,
            id,
        })),
        // PDF-originated Markdown must not make an EPUB reader contact a
        // remote image host. Extracted images are packaged separately below;
        // keep only the Markdown image's accessible label as ordinary text.
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        other => Some(other),
    });
    let mut output = String::with_capacity(markdown.len() + markdown.len() / 8);
    html::push_html(&mut output, events);
    output
}

fn safe_link_destination(destination: CowStr<'_>) -> CowStr<'_> {
    if is_safe_link_destination(&destination) {
        destination
    } else {
        CowStr::Borrowed("")
    }
}

fn is_safe_link_destination(destination: &str) -> bool {
    let destination = destination.trim();
    if destination.chars().any(char::is_control) {
        return false;
    }
    let Some(colon) = destination.find(':') else {
        return true;
    };
    let first_path_separator = destination.find(['/', '#', '?']).unwrap_or(usize::MAX);
    if colon > first_path_separator {
        return true;
    }
    matches!(
        destination[..colon].to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto"
    )
}

fn xhtml_document(title: &str, language: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{}\" lang=\"{}\">\n<head>\n<meta charset=\"UTF-8\"/>\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../styles.css\"/>\n</head>\n<body>\n{}</body>\n</html>\n",
        escape_xml(language),
        escape_xml(language),
        escape_xml(title),
        body
    )
}

fn normalized_title(title: &str) -> String {
    let sanitized: String = title
        .trim()
        .chars()
        .filter(|character| is_valid_xml_character(*character))
        .take(300)
        .collect();
    if sanitized.trim().is_empty() {
        "Converted document".to_string()
    } else {
        sanitized
    }
}

fn normalized_language(language: &str) -> String {
    let language = language.trim();
    if language.is_empty()
        || language.len() > 35
        || !language
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        DEFAULT_LANGUAGE.to_string()
    } else {
        language.to_string()
    }
}

fn document_identifier(input: &[u8]) -> String {
    // Stable FNV-1a identifiers avoid random-number and clock dependencies in
    // both native and wasm32 builds. EPUB requires uniqueness, not cryptography.
    let hash = input.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("urn:paprika:{hash:016x}")
}

fn first_heading(html: &str) -> Option<String> {
    let mut first: Option<(usize, String)> = None;
    for level in 1..=6 {
        let open = format!("<h{level}>");
        let close = format!("</h{level}>");
        let Some(start) = html.find(&open) else {
            continue;
        };
        let content_start = start + open.len();
        let Some(relative_end) = html[content_start..].find(&close) else {
            continue;
        };
        let text = strip_markup(&html[content_start..content_start + relative_end]);
        let meaningful = text
            .chars()
            .filter(|character| character.is_alphanumeric())
            .count()
            >= 2;
        if meaningful && first.as_ref().is_none_or(|(position, _)| start < *position) {
            first = Some((start, text));
        }
    }
    first.map(|(_, heading)| heading.chars().take(120).collect())
}

fn visible_text_len(html: &str) -> usize {
    strip_markup(html)
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
}

fn strip_markup(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside_tag = false;
    for character in html.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                text.push(' ');
            }
            _ if !inside_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_invalid_xml_characters(value: &mut String) {
    value.retain(is_valid_xml_character);
}

fn is_valid_xml_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value
        .chars()
        .filter(|character| is_valid_xml_character(*character))
    {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_first_meaningful_heading_in_document_order() {
        let html = "<h2>…</h2><p>Lead</p><h3>First &amp; best</h3><h1>Later</h1>";
        assert_eq!(first_heading(html).as_deref(), Some("First & best"));
    }

    #[test]
    fn treats_pdf_supplied_html_as_text() {
        let html = markdown_to_xhtml("<script>alert('x')</script>");
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn strips_active_link_schemes_from_pdf_text() {
        let html = markdown_to_xhtml("[unsafe](javascript:alert(1)) [safe](https://example.com)");
        assert!(!html.contains("javascript:"));
        assert!(html.contains("href=\"https://example.com\""));
    }

    #[test]
    fn renders_remote_markdown_images_as_plain_alt_text() {
        let html = markdown_to_xhtml("![Figure label](//attacker.invalid/pixel.png)");
        assert!(!html.contains("<img"));
        assert!(!html.contains("attacker.invalid"));
        assert!(html.contains("Figure label"));
    }

    #[test]
    fn removes_xml_forbidden_metadata_characters() {
        assert_eq!(normalized_title("A\u{1} B"), "A B");
        assert_eq!(normalized_title("\u{1}"), "Converted document");
        assert_eq!(escape_xml("A\u{1} & B"), "A &amp; B");
    }

    #[test]
    fn bounds_encoded_image_buffers() {
        let mut output = BoundedBuffer::new(3);
        assert!(output.write_all(b"four").is_err());
        assert!(output.limit_exceeded);
        assert!(output.into_inner().is_empty());
    }

    #[test]
    fn measures_image_overlap_for_crop_deduplication() {
        let image = Rect::new(10.0, 10.0, 100.0, 100.0);
        assert_eq!(
            overlap_fraction(image, Rect::new(10.0, 10.0, 100.0, 100.0)),
            1.0
        );
        assert_eq!(
            overlap_fraction(image, Rect::new(200.0, 200.0, 10.0, 10.0)),
            0.0
        );
        assert!(
            (overlap_fraction(image, Rect::new(60.0, 10.0, 100.0, 100.0)) - 0.5).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn tightens_semantic_exclusion_to_graphic_bounds() {
        let coarse = Rect::new(59.0, 524.0, 227.0, 225.0);
        let graphics = tighten_regions_from_graphics(
            &[coarse],
            &[
                Rect::new(60.0, 525.0, 225.0, 72.0),
                // A page-sized background must not expand the exclusion.
                Rect::new(0.0, 0.0, 612.0, 792.0),
            ],
            &[],
        );
        assert_eq!(graphics.len(), 1);
        assert!(graphics[0].y + graphics[0].height < 600.0);
        let prose_above_figure = Rect::new(60.0, 650.0, 220.0, 9.0);
        assert_eq!(overlap_fraction(prose_above_figure, graphics[0]), 0.0);
    }

    fn test_span(text: &str, x: f32, y: f32, width: f32, font: &str) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            bbox: Rect::new(x, y, width, 10.0),
            font_name: font.to_string(),
            font_size: 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn detects_formula_heavy_pages_without_overreacting_to_equations() {
        let mut dense = Vec::new();
        for index in 0..190 {
            dense.push(test_span(
                "ordinary prose",
                40.0,
                740.0 - index as f32,
                120.0,
                "Times",
            ));
        }
        for index in 0..110 {
            dense.push(test_span(
                "𝑥=",
                220.0,
                740.0 - index as f32,
                10.0,
                "LibertineMathMI",
            ));
        }
        assert!(is_math_dense_candidate(&dense, &[]));
        assert!(math_extraction_is_unreliable(
            &dense,
            &[],
            "<p>Extracted prose without mathematical relations.</p>"
        ));
        let preserved_operators = format!("<p>{}</p>", "=".repeat(60));
        assert!(!math_extraction_is_unreliable(
            &dense,
            &[],
            &preserved_operators
        ));

        let mut fragmented = dense.clone();
        for span in fragmented.iter_mut().skip(190) {
            span.text = "𝑥".to_string();
        }
        for span in fragmented.iter_mut().skip(190).take(15) {
            span.font_size = 6.0;
        }
        assert!(math_extraction_is_unreliable(
            &fragmented,
            &[],
            "<p>Flattened variables without script structure.</p>"
        ));
        assert!(!is_math_dense_candidate(
            &dense,
            &[Rect::new(200.0, 600.0, 40.0, 160.0)]
        ));

        let equation_page: Vec<_> = (0..80)
            .map(|index| test_span("𝑥", 220.0, 740.0 - index as f32, 6.0, "LibertineMathMI"))
            .collect();
        assert!(!is_math_dense_candidate(&equation_page, &[]));
    }

    #[test]
    fn splits_visual_columns_without_gaps_or_center_loss() {
        let mut spans = Vec::new();
        for row in 0..10 {
            spans.push(test_span(
                "Long ordinary text in the left source column",
                24.0,
                720.0 - row as f32 * 70.0,
                240.0,
                "Times",
            ));
            spans.push(test_span(
                "Long ordinary text in the right source column",
                348.0,
                720.0 - row as f32 * 70.0,
                240.0,
                "Times",
            ));
        }
        let regions = visual_page_regions(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        assert_eq!(regions.len(), 4);
        assert!(regions[0].1.starts_with("left column"));
        assert!(regions[2].1.starts_with("right column"));
        assert!(regions[0].0.x + regions[0].0.width > 306.0);
        assert!(regions[2].0.x < 306.0);
        assert_eq!(regions[0].0.y, regions[1].0.y + regions[1].0.height);
        assert_eq!(regions[2].0.y, regions[3].0.y + regions[3].0.height);

        let parallel_columns = spans.clone();
        spans.push(test_span(
            "𝑓(𝑥) = 𝑦",
            100.0,
            400.0,
            412.0,
            "LibertineMathMI",
        ));
        let full_width = visual_page_regions(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        assert_eq!(full_width.len(), 1);
        assert_eq!(full_width[0].0, Rect::new(0.0, 0.0, 612.0, 792.0));

        let diagram = [
            Rect::new(140.0, 360.0, 100.0, 80.0),
            Rect::new(250.0, 360.0, 100.0, 80.0),
            Rect::new(360.0, 360.0, 100.0, 80.0),
        ];
        let full_width = visual_page_regions(
            &parallel_columns,
            Rect::new(0.0, 0.0, 612.0, 792.0),
            &diagram,
        );
        assert_eq!(full_width.len(), 1);
    }

    #[test]
    fn detects_right_numbered_display_equation() {
        let mut spans = vec![
            test_span(
                "Body prose before the display equation.",
                108.0,
                350.0,
                396.0,
                "Times",
            ),
            test_span("Attention(", 220.0, 311.0, 46.0, "CMR10"),
            test_span("Q, K, V", 266.0, 311.0, 31.0, "CMMI10"),
            test_span(") = softmax(", 299.0, 311.0, 55.0, "CMR10"),
            test_span("QK", 356.0, 318.0, 18.0, "CMMI10"),
            test_span("d", 366.0, 303.0, 5.0, "CMMI10"),
            test_span(")V", 380.0, 311.0, 10.0, "CMMI10"),
            test_span("(1)", 493.0, 311.0, 12.0, "Times"),
            test_span(
                "Body prose after the display equation.",
                108.0,
                270.0,
                396.0,
                "Times",
            ),
        ];
        for offset in 0..4 {
            spans.push(test_span(
                "Additional ordinary body text for font statistics.",
                108.0,
                500.0 + offset as f32 * 12.0,
                396.0,
                "Times",
            ));
        }
        let plans = find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].anchor, "(1)");
        assert!(
            plans[0]
                .exclusion_rects
                .iter()
                .all(|bbox| overlap_fraction(spans[7].bbox, *bbox) < 0.5)
        );
    }

    #[test]
    fn partitions_adjacent_numbered_equations() {
        let mut spans = vec![
            test_span("f(", 210.0, 410.0, 12.0, "CMR10"),
            test_span("x", 222.0, 410.0, 7.0, "CMMI10"),
            test_span(") = 1", 229.0, 410.0, 35.0, "CMR10"),
            test_span("(1)", 493.0, 410.0, 12.0, "Times"),
            test_span("g(", 210.0, 395.0, 12.0, "CMR10"),
            test_span("x", 222.0, 395.0, 7.0, "CMMI10"),
            test_span(") = 2", 229.0, 395.0, 35.0, "CMR10"),
            test_span("(2)", 493.0, 395.0, 12.0, "Times"),
        ];
        for offset in 0..8 {
            spans.push(test_span(
                "Additional ordinary body text for font statistics.",
                108.0,
                500.0 + offset as f32 * 12.0,
                396.0,
                "Times",
            ));
        }
        let plans = find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        assert_eq!(
            plans
                .iter()
                .map(|plan| plan.anchor.as_str())
                .collect::<Vec<_>>(),
            vec!["(1)", "(2)"]
        );
        assert!(overlap_fraction(plans[0].render_bbox, plans[1].render_bbox) < 0.1);
    }

    #[test]
    fn rejects_inline_math_and_numbered_list_items() {
        let spans = vec![
            test_span(
                "The loss L(x) = 3 is minimized in this example.",
                108.0,
                400.0,
                396.0,
                "Times",
            ),
            test_span("(1)", 108.0, 380.0, 12.0, "Times"),
            test_span("First ordinary list item", 128.0, 380.0, 180.0, "Times"),
        ];
        assert!(find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]).is_empty());
    }

    #[test]
    fn detects_isolated_unnumbered_equation() {
        let spans = vec![
            test_span(
                "Body prose before the display equation.",
                108.0,
                680.0,
                396.0,
                "Times",
            ),
            test_span("MultiHead(", 187.0, 637.0, 50.0, "CMR10"),
            test_span("Q, K, V", 237.0, 637.0, 31.0, "CMMI10"),
            test_span(") = Concat(head", 271.0, 637.0, 72.0, "CMR10"),
            test_span("1, ..., head", 343.0, 637.0, 50.0, "CMMI10"),
            test_span("h)W", 394.0, 637.0, 17.0, "CMMI10"),
            test_span(
                "Body prose after the display equation.",
                108.0,
                590.0,
                396.0,
                "Times",
            ),
        ];
        let plans = find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].anchor, "MultiHead(");
    }

    #[test]
    fn rejects_equations_inside_figures_or_tables() {
        let spans = vec![
            test_span("FFN(", 227.0, 215.0, 24.0, "CMR10"),
            test_span("x", 251.0, 215.0, 7.0, "CMMI10"),
            test_span(") = max(0,", 258.0, 215.0, 55.0, "CMR10"),
            test_span("xW", 313.0, 215.0, 20.0, "CMMI10"),
            test_span("(2)", 493.0, 215.0, 12.0, "Times"),
        ];
        let occupied = Rect::new(200.0, 190.0, 320.0, 60.0);
        assert!(
            find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[occupied])
                .is_empty()
        );
    }

    #[test]
    fn skips_unnumbered_equations_on_algorithm_pages() {
        let spans = vec![
            test_span("Algorithm 2:", 60.0, 500.0, 70.0, "Times"),
            test_span("score(", 187.0, 400.0, 40.0, "CMR10"),
            test_span("x", 227.0, 400.0, 7.0, "CMMI10"),
            test_span(") = 1", 234.0, 400.0, 35.0, "CMR10"),
        ];
        assert!(find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[]).is_empty());
    }

    #[test]
    fn requires_a_unique_equation_anchor() {
        let image = PageImage {
            href: "images/equation.png".to_string(),
            bytes: Vec::new(),
            alt: "Display equation".to_string(),
            placement: ImagePlacement::EquationAnchor("(1)".to_string()),
        };
        assert!(equation_anchors_are_unique(
            "<p>Formula: (1)</p>",
            std::slice::from_ref(&image)
        ));
        assert!(!equation_anchors_are_unique(
            "<p>No anchor</p>",
            std::slice::from_ref(&image)
        ));
        assert!(!equation_anchors_are_unique(
            "<p>(1)</p><p>Again (1)</p>",
            &[image]
        ));

        let colliding = [
            PageImage {
                href: "images/first.png".to_string(),
                bytes: Vec::new(),
                alt: "First equation".to_string(),
                placement: ImagePlacement::EquationAnchor("MultiHead".to_string()),
            },
            PageImage {
                href: "images/second.png".to_string(),
                bytes: Vec::new(),
                alt: "Second equation".to_string(),
                placement: ImagePlacement::EquationAnchor("Head".to_string()),
            },
        ];
        assert!(!equation_anchors_are_unique("<p>MultiHead</p>", &colliding));
    }

    #[test]
    fn replaces_formatted_caption_with_accessible_figure() {
        let mut body = "<p>Before Figure 1.</p>\n<p><strong>Figure</strong> <strong>1:</strong> Sample graph</p>\n<p>After.</p>".to_string();
        assert!(replace_caption_paragraph_with_figure(
            &mut body,
            "Figure 1:",
            "../images/figure.png",
            "Figure 1: Sample graph"
        ));
        assert!(body.contains(
            "<figure class=\"figure-fallback\"><img src=\"../images/figure.png\" alt=\"Figure 1: Sample graph\"/><figcaption><strong>Figure</strong> <strong>1:</strong> Sample graph</figcaption></figure>"
        ));
        assert_eq!(body.matches("Figure</strong> <strong>1:").count(), 1);
        assert!(body.ends_with("<p>After.</p>"));
    }

    #[test]
    fn replaces_equation_anchor_after_lead_prose() {
        let mut body = "<p>According to the formula: (3)</p>\n<p>After.</p>".to_string();
        assert!(replace_equation_anchor_with_image(
            &mut body,
            "(3)",
            "../images/equation.png",
            "Display equation (3)"
        ));
        assert!(body.contains("<p>According to the formula:</p>"));
        assert!(body.contains(
            "<figure class=\"equation-fallback\"><img src=\"../images/equation.png\" alt=\"Display equation (3)\"/></figure>"
        ));
        assert!(!body.contains("formula: (3)"));
    }

    #[test]
    fn formats_algorithm_steps_as_selectable_lines() {
        let mut html = "<p><strong>Algorithm</strong> <strong>2:</strong> Worker <strong>1 do</strong> work</p>\n<p><strong>2</strong> done **</p>\n<p>Ordinary prose.</p>".to_string();
        enhance_algorithm_blocks(&mut html);
        assert!(html.starts_with("<pre class=\"algorithm\">"));
        assert!(html.contains("\n<strong>1 do</strong>"));
        assert!(html.contains("\n<strong>2</strong>"));
        assert!(!html.contains("**"));
        assert!(html.ends_with("\n<p>Ordinary prose.</p>"));
    }

    #[test]
    fn rejects_invalid_language_metadata() {
        assert_eq!(normalized_language(" en-US "), "en-US");
        assert_eq!(normalized_language("en<script>"), "en");
    }

    #[test]
    fn emits_valid_xhtml_shell_and_escapes_metadata() {
        let xhtml = xhtml_document("A & B", "en", "<p>Body</p>");
        assert!(xhtml.contains("<title>A &amp; B</title>"));
        assert!(xhtml.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
        assert!(xhtml.contains("<p>Body</p>"));
    }

    #[test]
    fn builds_epub_three_archive_in_memory() {
        let page = SemanticPage {
            number: 1,
            title: "Introduction".to_string(),
            html: "<h1>Introduction</h1><p>Select me</p>".to_string(),
            images: Vec::new(),
            has_text: true,
        };
        let bytes = package_epub("Test", "en", "urn:paprika:test", vec![page]).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert!(
            bytes
                .windows(b"application/epub+zip".len())
                .any(|window| window == b"application/epub+zip")
        );
        let epub = Epub::read(std::io::Cursor::new(bytes)).unwrap();
        let chapter = epub.read_resource_str("text/page-0001.xhtml").unwrap();
        assert!(chapter.contains("Select me"));
    }
}
