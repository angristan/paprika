//! Semantic PDF-to-EPUB conversion shared by the native and browser clients.
//!
//! The converter keeps born-digital text selectable and lets the EPUB reader
//! perform pagination. It also carries useful raster image XObjects across as
//! EPUB resources. Pages without a usable text layer are called out explicitly

mod buffer;
mod equations;
mod extraction;
mod figures;
mod geometry;
mod images;
mod math;
mod model;
mod packaging;
mod sanitization;

use hayro::hayro_syntax::Pdf as SourcePdf;
use pdf_oxide::PdfDocument;
use pdf_oxide::converters::{ConversionOptions, ReadingOrderMode};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::RectFilterMode;
use thiserror::Error;

use equations::{AssetBudget, EquationCrops, collect_equation_crops};
use extraction::{
    account_rendered_xhtml, collect_repeated_running_text, equation_anchors_are_unique,
    extract_page_xhtml,
};
use figures::{FigureCrops, collect_figure_crops, page_may_have_figure_caption};
use images::{
    ImageDecodeBudget, PageImageCollection, account_image_objects, collect_page_images,
    source_page_bounds,
};
use math::{
    PageVisualContext, collect_formula_page_crops, is_math_dense_candidate,
    math_extraction_is_unreliable, trustworthy_prose_html,
};
use model::SemanticPage;
use packaging::{build_epub_preview, package_epub};
use sanitization::{
    document_identifier, enhance_algorithm_blocks, first_heading, no_text_warning,
    normalized_language, normalized_title, visible_text_len,
};

const DEFAULT_LANGUAGE: &str = "en";
const MAX_IMAGE_PIXELS: u64 = 4_000_000;
const MAX_IMAGE_DECODE_PIXELS: u64 = 32_000_000;
const MAX_IMAGE_DECODE_PIXELS_PER_PAGE: u64 = 64_000_000;
const MAX_IMAGE_DECODE_PIXELS_TOTAL: u64 = 512_000_000;
const MAX_IMAGE_OBJECTS_PER_PAGE: usize = 512;
const MAX_IMAGE_OBJECTS_TOTAL: usize = 16_384;
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
    /// Optional bounded browser preview generated from the same semantic pages.
    pub preview_limits: Option<EpubPreviewLimits>,
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
            preview_limits: None,
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
    pub preview: Option<EpubPreview>,
}

#[derive(Clone, Copy, Debug)]
pub struct EpubPreviewLimits {
    pub max_chapters: usize,
    pub max_xhtml_bytes: usize,
    pub max_asset_bytes: usize,
    pub max_assets: usize,
}

#[derive(Debug)]
pub struct EpubPreview {
    pub stylesheet: String,
    pub chapters: Vec<EpubPreviewChapter>,
    pub assets: Vec<EpubPreviewAsset>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct EpubPreviewChapter {
    pub source_page: usize,
    pub title: String,
    pub href: String,
    pub xhtml: String,
}

#[derive(Debug)]
pub struct EpubPreviewAsset {
    pub href: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
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
    #[error("source page {page} contains {objects} image objects; the safety limit is {limit}")]
    TooManyImageObjects {
        page: usize,
        objects: usize,
        limit: usize,
    },
    #[error("the PDF contains {objects} image objects; the safety limit is {limit}")]
    TooManyImageObjectsTotal { objects: usize, limit: usize },
    #[error(
        "decoding images from source page {page} would exceed the {limit} megapixel safety limit"
    )]
    PageImageDecodeLimit { page: usize, limit: u64 },
    #[error("decoding source images would exceed the {limit} megapixel safety limit")]
    ImageDecodeLimit { limit: u64 },
}

/// Convert borrowed PDF bytes to a reflowable EPUB 3 archive entirely in memory.
pub fn convert_pdf_to_epub(input: &[u8], options: EpubOptions) -> Result<ConvertedEpub, EpubError> {
    convert_pdf_to_epub_owned(input.to_vec(), options)
}

/// Convert owned PDF bytes without retaining an extra full-size input copy.
pub fn convert_pdf_to_epub_owned(
    input: Vec<u8>,
    options: EpubOptions,
) -> Result<ConvertedEpub, EpubError> {
    let input_len = input.len();
    let identifier = document_identifier(&input);
    // Hayro and pdf_oxide each own their parser storage. Keep the second copy
    // only when visual fallbacks are enabled and the source is small enough.
    let render_input = (options.include_images && input_len <= MAX_FIGURE_RENDER_INPUT_BYTES)
        .then(|| input.clone());
    let document =
        PdfDocument::from_bytes(input).map_err(|error| EpubError::Parse(error.to_string()))?;
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

    let title = normalized_title(&options.title);
    let language = normalized_language(&options.language);
    let extraction_options = ConversionOptions {
        preserve_layout: false,
        detect_headings: true,
        extract_tables: true,
        include_images: false,
        // pdf_oxide's legacy running-header pass rescans every page for every
        // chapter. Artifact filtering remains enabled without that O(pages²)
        // pass, and avoids multi-minute conversions on longer documents.
        strip_running_headers_footers: false,
        reading_order_mode: ReadingOrderMode::ColumnAware,
        expand_ligatures: true,
        include_artifacts: false,
        ..Default::default()
    };

    let mut semantic_pages = Vec::with_capacity(page_count);
    let mut warnings = Vec::new();
    let repeated_running_text = collect_repeated_running_text(&document, page_count);
    let render_document = match render_input {
        Some(bytes) => match SourcePdf::new(bytes) {
            Ok(source) => Some(source),
            Err(_) => {
                warnings.push(
                    "Visual figure, equation, and math-page fallbacks are unavailable because the PDF renderer could not open the source."
                        .to_string(),
                );
                None
            }
        },
        None => {
            if options.include_images && input_len > MAX_FIGURE_RENDER_INPUT_BYTES {
                warnings.push(
                    "Visual figure, equation, and math-page fallbacks were skipped because the source exceeds 32 MiB."
                        .to_string(),
                );
            }
            None
        }
    };
    let mut asset_bytes = 0usize;
    let mut semantic_bytes = 0usize;
    let mut image_count = 0usize;
    let mut text_pages = 0usize;
    let mut image_decode_budget = ImageDecodeBudget::default();

    for page_index in 0..page_count {
        let page_asset_bytes = asset_bytes;
        let page_image_handles = if options.include_images {
            document
                .page_image_handles(page_index)
                .unwrap_or_else(|error| {
                    warnings.push(format!(
                        "Could not inspect images from page {}: {error}",
                        page_index + 1
                    ));
                    Vec::new()
                })
        } else {
            Vec::new()
        };
        account_image_objects(
            &mut image_decode_budget,
            page_index,
            page_image_handles.len(),
        )?;
        let page_image_bounds: Vec<Rect> =
            page_image_handles.iter().map(|image| image.bbox).collect();
        let page_spans = if options.include_images {
            document.extract_spans(page_index).unwrap_or_else(|error| {
                warnings.push(format!(
                    "Could not inspect positioned text from page {}: {error}",
                    page_index + 1
                ));
                Vec::new()
            })
        } else {
            Vec::new()
        };
        let crops = if options.include_images && page_may_have_figure_caption(&page_spans) {
            render_document
                .as_ref()
                .map(|source| {
                    collect_figure_crops(
                        source,
                        &document,
                        page_index,
                        &page_image_bounds,
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
        if let Some(source) = render_document.as_ref().filter(|_| options.include_images)
            && is_math_dense_candidate(&page_spans, &crops.regions)
        {
            let mut probe_options = extraction_options.clone();
            probe_options.exclude_regions = crops.text_exclusions.clone();
            probe_options.exclude_regions_mode = RectFilterMode::MinOverlap(0.5);
            let (_, probe_html) = extract_page_xhtml(
                &document,
                page_index,
                &probe_options,
                &repeated_running_text,
            )?;
            let should_fallback =
                math_extraction_is_unreliable(&page_spans, &crops.regions, &probe_html);
            if should_fallback {
                let mut fallback_asset_bytes = page_asset_bytes;
                let images = collect_formula_page_crops(
                    source,
                    &document,
                    PageVisualContext {
                        index: page_index,
                        spans: &page_spans,
                        image_bounds: &page_image_bounds,
                    },
                    &mut fallback_asset_bytes,
                    options.max_asset_bytes,
                    &mut warnings,
                )?;
                if !images.is_empty() {
                    let preserved_prose = trustworthy_prose_html(&probe_html);
                    let has_text = visible_text_len(&preserved_prose) >= 20;
                    let mut html = format!(
                        "<p class=\"conversion-note\">Source page {} contains dense mathematical layout that could not be represented reliably as selectable text. It is preserved visually in reading order.</p>\n",
                        page_index + 1
                    );
                    if has_text {
                        html.push_str(
                            "<section class=\"preserved-prose\" aria-label=\"Recovered selectable prose\">\n",
                        );
                        html.push_str(&preserved_prose);
                        html.push_str("</section>\n");
                        text_pages += 1;
                    }
                    asset_bytes = fallback_asset_bytes;
                    image_count += images.len();
                    warnings.push(format!(
                        "Page {} has dense mathematical layout and was preserved as visual columns.",
                        page_index + 1
                    ));
                    let page = SemanticPage {
                        number: page_index + 1,
                        title: first_heading(&preserved_prose)
                            .unwrap_or_else(|| format!("Page {}", page_index + 1)),
                        html,
                        images,
                        has_text,
                    };
                    account_rendered_xhtml(
                        &page,
                        &language,
                        &mut semantic_bytes,
                        options.max_semantic_bytes,
                    )?;
                    semantic_pages.push(page);
                    continue;
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
                        PageVisualContext {
                            index: page_index,
                            spans: &page_spans,
                            image_bounds: &page_image_bounds,
                        },
                        &crops.regions,
                        AssetBudget {
                            used: asset_bytes,
                            maximum: options.max_asset_bytes,
                        },
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
        let (_, mut html) = extract_page_xhtml(
            &document,
            page_index,
            &page_extraction_options,
            &repeated_running_text,
        )?;
        if !equation_anchors_are_unique(&html, &equations.images) {
            warnings.push(format!(
                "Display equations from page {} remained as text because their reading position was ambiguous.",
                page_index + 1
            ));
            equations = EquationCrops::default();
            page_extraction_options.exclude_regions = crops.text_exclusions.clone();
            (_, html) = extract_page_xhtml(
                &document,
                page_index,
                &page_extraction_options,
                &repeated_running_text,
            )?;
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
        let images = if options.include_images {
            let FigureCrops {
                mut images,
                regions,
                ..
            } = crops;
            images.extend(equations.images);
            images.extend(collect_page_images(
                &page_image_handles,
                PageImageCollection {
                    index: page_index,
                    excluded_regions: &regions,
                    image_only: !has_text,
                    bounds: source_page_bounds(&document, page_index),
                },
                &mut image_decode_budget,
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
        let page = SemanticPage {
            number: page_index + 1,
            title,
            html,
            images,
            has_text,
        };
        account_rendered_xhtml(
            &page,
            &language,
            &mut semantic_bytes,
            options.max_semantic_bytes,
        )?;
        semantic_pages.push(page);
    }

    if text_pages == 0 {
        warnings.push(no_text_warning(image_count).to_string());
    }

    let preview = options
        .preview_limits
        .map(|limits| build_epub_preview(&semantic_pages, &language, limits))
        .transpose()?;
    let bytes = package_epub(
        &title,
        &language,
        &identifier,
        semantic_pages,
        options.max_semantic_bytes,
        options.max_output_bytes,
    )?;

    Ok(ConvertedEpub {
        bytes,
        source_pages: page_count,
        text_pages,
        image_count,
        warnings,
        preview,
    })
}

/// Parse, validate, and canonicalize a BCP 47 language tag.
pub fn normalize_language_tag(language: &str) -> Option<String> {
    sanitization::normalize_language_tag(language)
}

#[cfg(test)]
mod tests;
