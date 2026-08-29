use paprika_core::OptimizationOptions;
use paprika_epub::{EpubOptions, convert_pdf_to_epub};
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

/// Convert a born-digital PDF to a reflowable EPUB 3 archive in memory.
///
/// The website calls this inside a Web Worker so extraction and packaging do
/// not block interaction on the main browser thread.
#[wasm_bindgen]
pub fn convert_pdf_to_epub_bytes(input: &[u8], title: String) -> Result<Vec<u8>, JsValue> {
    enforce_input_limit(input)?;
    let result = convert_pdf_to_epub(
        input,
        EpubOptions {
            title,
            max_pages: MAX_BROWSER_PAGES,
            // Keep browser output bounded even when a PDF contains many large
            // image XObjects. The native CLI has a larger default allowance.
            max_asset_bytes: 56 * 1024 * 1024,
            max_semantic_bytes: 24 * 1024 * 1024,
            max_output_bytes: 96 * 1024 * 1024,
            ..Default::default()
        },
    )
    .map_err(js_error)?;
    Ok(result.bytes)
}

/// Produce the legacy raster PDF fallback in memory.
#[wasm_bindgen]
pub fn optimize_pdf_bytes(input: &[u8], options: JsValue) -> Result<Vec<u8>, JsValue> {
    enforce_input_limit(input)?;
    let options: OptimizationOptions = serde_wasm_bindgen::from_value(options).map_err(js_error)?;
    options.validate().map_err(js_error)?;
    let result =
        paprika_pdf::optimize_pdf_with_limits(input, options, paprika_pdf::PdfLimits::browser())
            .map_err(js_error)?;
    Ok(result.bytes)
}

fn enforce_input_limit(input: &[u8]) -> Result<(), JsValue> {
    if input.len() > MAX_BROWSER_INPUT_BYTES {
        return Err(JsValue::from_str(
            "This browser build accepts PDFs up to 64 MiB. Use the CLI for larger documents.",
        ));
    }
    Ok(())
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
