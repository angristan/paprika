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
use pulldown_cmark::{CowStr, Event, Options as MarkdownOptions, Parser, Tag, TagEnd, html};
use rbook::Epub;
use rbook::epub::EpubChapter;
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
}

#[derive(Debug, Default)]
struct FigureCrops {
    images: Vec<PageImage>,
    regions: Vec<Rect>,
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
                "Captioned vector-figure crops were skipped because the source exceeds 32 MiB."
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
        let markdown = document
            .to_markdown(page_index, &extraction_options)
            .map_err(|error| EpubError::Extract {
                page: page_index + 1,
                message: error.to_string(),
            })?;
        let remaining_semantic_bytes = options.max_semantic_bytes.saturating_sub(semantic_bytes);
        if markdown.len() > remaining_semantic_bytes {
            return Err(EpubError::SemanticTooLarge {
                limit: options.max_semantic_bytes / (1024 * 1024),
            });
        }
        let mut html = markdown_to_xhtml(&markdown);
        strip_invalid_xml_characters(&mut html);
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
            let crops = render_document
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
                .unwrap_or_default();
            let mut images = crops.images;
            images.extend(collect_page_images(
                &document,
                page_index,
                &crops.regions,
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
    let mut images = Vec::with_capacity(captions.len());
    let mut regions = Vec::with_capacity(captions.len());
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

        let x = ((crop_left - llx) * x_scale).floor().max(0.0) as u32;
        let y = ((ury - crop_top) * y_scale).floor().max(0.0) as u32;
        let crop_width = ((crop_right - crop_left) * x_scale)
            .ceil()
            .min(width.saturating_sub(x) as f32) as u32;
        let crop_height = ((crop_top - crop_bottom) * y_scale)
            .ceil()
            .min(height.saturating_sub(y) as f32) as u32;
        if crop_width < 80 || crop_height < 48 {
            continue;
        }

        let crop = image::imageops::crop_imm(&rendered, x, y, crop_width, crop_height).to_image();
        let mut cursor = std::io::Cursor::new(Vec::new());
        if let Err(error) =
            image::DynamicImage::ImageRgba8(crop).write_to(&mut cursor, image::ImageFormat::Png)
        {
            warnings.push(format!(
                "Could not encode figure {} from page {}: {error}",
                caption_index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = cursor.into_inner();
        account_asset(total_bytes, bytes.len(), max_bytes)?;
        regions.push(Rect::new(
            crop_left,
            crop_bottom,
            crop_right - crop_left,
            crop_top - crop_bottom,
        ));
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-figure-{:02}.png",
                page_index + 1,
                caption_index + 1
            ),
            bytes,
            // Several captions in adjacent columns can share a baseline. The
            // crop remains useful, but attaching the wrong specific caption is
            // worse than a source-page label.
            alt: if captions.len() == 1 {
                caption.text.clone()
            } else {
                format!("Figure crop from source page {}", page_index + 1)
            },
        });
    }
    Ok(FigureCrops { images, regions })
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
        if !page.images.is_empty() {
            body.push_str(
                "<section class=\"page-images\" aria-label=\"Images from this source page\">\n",
            );
        }
        for image in page.images {
            body.push_str(&format!(
                "<figure><img src=\"../{}\" alt=\"{}\"/><figcaption>{}</figcaption></figure>\n",
                escape_xml(&image.href),
                escape_xml(&image.alt),
                escape_xml(&image.alt)
            ));
            editor = editor.resource((image.href, image.bytes));
        }
        if body.contains("<section class=\"page-images\"") {
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
