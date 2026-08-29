use paprika_core::OptimizationOptions;
use paprika_epub::{
    EpubOptions, EpubPreview, EpubPreviewLimits, convert_pdf_to_epub_owned, normalize_language_tag,
};
use wasm_bindgen::prelude::*;

const MAX_BROWSER_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_BROWSER_PAGES: usize = 500;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Return default options as a plain JavaScript object.
#[wasm_bindgen]
pub fn default_options() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&OptimizationOptions::default()).map_err(js_error)
}

/// Read page count without rendering the PDF.
#[wasm_bindgen]
pub fn inspect_pdf(input: &[u8]) -> Result<usize, JsValue> {
    enforce_input_limit(input)?;
    let pages = paprika_pdf::page_count(input).map_err(js_error)?;
    if pages > MAX_BROWSER_PAGES {
        return Err(JsValue::from_str(
            "This browser build accepts at most 500 source pages at a time.",
        ));
    }
    Ok(pages)
}

#[derive(serde::Serialize)]
struct PreviewManifest<'a> {
    stylesheet: &'a str,
    chapters: Vec<PreviewChapter<'a>>,
    assets: Vec<PreviewAsset<'a>>,
    truncated: bool,
}

#[derive(serde::Serialize)]
struct PreviewChapter<'a> {
    source_page: usize,
    title: &'a str,
    href: &'a str,
    xhtml: &'a str,
}

#[derive(serde::Serialize)]
struct PreviewAsset<'a> {
    index: usize,
    href: &'a str,
    media_type: &'a str,
}

/// Browser-owned EPUB result. Large byte buffers are taken exactly once so
/// JavaScript can transfer them without base64 or JSON duplication.
#[wasm_bindgen]
pub struct BrowserEpubConversion {
    output: Option<Vec<u8>>,
    source_pages: usize,
    text_pages: usize,
    image_count: usize,
    warnings: Vec<String>,
    preview: Option<EpubPreview>,
}

#[wasm_bindgen]
impl BrowserEpubConversion {
    #[wasm_bindgen(getter)]
    pub fn source_pages(&self) -> usize {
        self.source_pages
    }

    #[wasm_bindgen(getter)]
    pub fn text_pages(&self) -> usize {
        self.text_pages
    }

    #[wasm_bindgen(getter)]
    pub fn image_count(&self) -> usize {
        self.image_count
    }

    pub fn warnings(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.warnings).map_err(js_error)
    }

    pub fn preview_manifest(&self) -> Result<JsValue, JsValue> {
        let Some(preview) = &self.preview else {
            return Ok(JsValue::NULL);
        };
        let manifest = PreviewManifest {
            stylesheet: &preview.stylesheet,
            chapters: preview
                .chapters
                .iter()
                .map(|chapter| PreviewChapter {
                    source_page: chapter.source_page,
                    title: &chapter.title,
                    href: &chapter.href,
                    xhtml: &chapter.xhtml,
                })
                .collect(),
            assets: preview
                .assets
                .iter()
                .enumerate()
                .map(|(index, asset)| PreviewAsset {
                    index,
                    href: &asset.href,
                    media_type: &asset.media_type,
                })
                .collect(),
            truncated: preview.truncated,
        };
        serde_wasm_bindgen::to_value(&manifest).map_err(js_error)
    }

    pub fn preview_asset_count(&self) -> usize {
        self.preview
            .as_ref()
            .map_or(0, |preview| preview.assets.len())
    }

    pub fn take_preview_asset(&mut self, index: usize) -> Result<Vec<u8>, JsValue> {
        let asset = self
            .preview
            .as_mut()
            .and_then(|preview| preview.assets.get_mut(index))
            .ok_or_else(|| JsValue::from_str("invalid EPUB preview asset index"))?;
        Ok(std::mem::take(&mut asset.bytes))
    }

    pub fn take_output(&mut self) -> Result<Vec<u8>, JsValue> {
        self.output
            .take()
            .ok_or_else(|| JsValue::from_str("EPUB output was already transferred"))
    }
}

/// Convert a born-digital PDF to EPUB plus a bounded browser preview.
#[wasm_bindgen]
pub fn convert_pdf_to_epub(
    input: Vec<u8>,
    title: String,
    language: String,
) -> Result<BrowserEpubConversion, JsValue> {
    enforce_input_limit(&input)?;
    let language = browser_language(&language)?;
    let result = convert_pdf_to_epub_owned(
        input,
        EpubOptions {
            title,
            language,
            max_pages: MAX_BROWSER_PAGES,
            // Keep browser output bounded even when a PDF contains many large
            // image XObjects. The native CLI has a larger default allowance.
            max_asset_bytes: 56 * 1024 * 1024,
            max_semantic_bytes: 24 * 1024 * 1024,
            max_output_bytes: 96 * 1024 * 1024,
            preview_limits: Some(EpubPreviewLimits {
                max_chapters: 12,
                max_xhtml_bytes: 2 * 1024 * 1024,
                max_asset_bytes: 8 * 1024 * 1024,
                max_assets: 48,
            }),
            ..Default::default()
        },
    )
    .map_err(js_error)?;
    Ok(BrowserEpubConversion {
        output: Some(result.bytes),
        source_pages: result.source_pages,
        text_pages: result.text_pages,
        image_count: result.image_count,
        warnings: result.warnings,
        preview: result.preview,
    })
}

/// Browser-owned raster result. The conversion reports its source page count
/// without reparsing the input in JavaScript before the real conversion.
#[wasm_bindgen]
pub struct BrowserPdfConversion {
    output: Option<Vec<u8>>,
    source_pages: usize,
    output_pages: usize,
}

#[wasm_bindgen]
impl BrowserPdfConversion {
    #[wasm_bindgen(getter)]
    pub fn source_pages(&self) -> usize {
        self.source_pages
    }

    #[wasm_bindgen(getter)]
    pub fn output_pages(&self) -> usize {
        self.output_pages
    }

    pub fn take_output(&mut self) -> Result<Vec<u8>, JsValue> {
        self.output
            .take()
            .ok_or_else(|| JsValue::from_str("PDF output was already transferred"))
    }
}

/// Produce the legacy raster PDF fallback in memory.
#[wasm_bindgen]
pub fn optimize_pdf_bytes(
    input: Vec<u8>,
    options: JsValue,
) -> Result<BrowserPdfConversion, JsValue> {
    enforce_input_limit(&input)?;
    let options: OptimizationOptions = serde_wasm_bindgen::from_value(options).map_err(js_error)?;
    options.validate().map_err(js_error)?;
    let result = paprika_pdf::optimize_pdf_with_limits_owned(
        input,
        options,
        paprika_pdf::PdfLimits::browser(),
    )
    .map_err(js_error)?;
    Ok(BrowserPdfConversion {
        output: Some(result.bytes),
        source_pages: result.source_pages,
        output_pages: result.output_pages,
    })
}

fn enforce_input_limit(input: &[u8]) -> Result<(), JsValue> {
    if input.len() > MAX_BROWSER_INPUT_BYTES {
        return Err(JsValue::from_str(
            "This browser build accepts PDFs up to 64 MiB. Use the CLI for larger documents.",
        ));
    }
    Ok(())
}

fn browser_language(language: &str) -> Result<String, JsValue> {
    normalize_language_tag(language).ok_or_else(|| {
        JsValue::from_str("Enter a valid BCP 47 language tag, such as en, fr, or pt-BR.")
    })
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
