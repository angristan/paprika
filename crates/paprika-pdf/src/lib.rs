//! Pure-Rust PDF adapter for Paprika.
//!
//! Hayro rasterizes source pages on native and `wasm32`; `pdf-writer` emits a
//! compact raster PDF. Keeping both sides in Rust gives the CLI and browser the
//! same behavior without native shared libraries.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as SourcePdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use miniz_oxide::deflate::{CompressionLevel, compress_to_vec_zlib};
use paprika_core::{DocumentOptimizer, OptimizationOptions, OptimizeError, RasterPage};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
use thiserror::Error;

const MAX_RASTER_EDGE: u32 = 16_384;
const NATIVE_SOURCE_PIXEL_LIMIT: u64 = 80_000_000;
const NATIVE_OUTPUT_PIXEL_LIMIT: u64 = 128_000_000;

/// Working-set limits for the in-memory raster pipeline.
#[derive(Clone, Copy, Debug)]
pub struct PdfLimits {
    pub max_source_pixels_per_page: u64,
    pub max_output_pixels: u64,
}

impl PdfLimits {
    pub const fn native() -> Self {
        Self {
            max_source_pixels_per_page: NATIVE_SOURCE_PIXEL_LIMIT,
            max_output_pixels: NATIVE_OUTPUT_PIXEL_LIMIT,
        }
    }

    pub const fn browser() -> Self {
        Self {
            // A rendered page temporarily exists as RGBA and RGB plus layout
            // crops, so 24 MP can already approach a few hundred MiB.
            max_source_pixels_per_page: 24_000_000,
            max_output_pixels: 32_000_000,
        }
    }
}

#[derive(Debug)]
pub struct OptimizedPdf {
    pub bytes: Vec<u8>,
    pub source_pages: usize,
    pub output_pages: usize,
}

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("could not parse the input PDF (encrypted PDFs are not supported)")]
    Parse,
    #[error("the PDF contains no pages")]
    Empty,
    #[error("source page {page} would rasterize to {width}x{height}; reduce source DPI")]
    RasterTooLarge {
        page: usize,
        width: u32,
        height: u32,
    },
    #[error("raster data returned by the PDF renderer was invalid")]
    InvalidRender,
    #[error(transparent)]
    Optimize(#[from] OptimizeError),
}

/// Reflow an in-memory PDF with conservative native-process limits.
pub fn optimize_pdf(input: &[u8], options: OptimizationOptions) -> Result<OptimizedPdf, PdfError> {
    optimize_pdf_with_limits(input, options, PdfLimits::native())
}

/// Reflow an in-memory PDF with an explicit raster working-set budget.
pub fn optimize_pdf_with_limits(
    input: &[u8],
    options: OptimizationOptions,
    limits: PdfLimits,
) -> Result<OptimizedPdf, PdfError> {
    options.validate()?;
    let pdf = SourcePdf::new(input.to_vec()).map_err(|_| PdfError::Parse)?;
    let pages = pdf.pages();
    if pages.is_empty() {
        return Err(PdfError::Empty);
    }

    let source_pages = pages.len();
    let mut optimizer = DocumentOptimizer::new_with_output_pixel_limit(
        options.clone(),
        Some(limits.max_output_pixels),
    )?;
    let cache = RenderCache::new();
    let interpreter_settings = InterpreterSettings::default();
    let scale = options.source_dpi as f32 / 72.0;

    for (index, page) in pages.iter().enumerate() {
        let (base_width, base_height) = page.render_dimensions();
        let width = (base_width * scale).ceil().max(1.0) as u32;
        let height = (base_height * scale).ceil().max(1.0) as u32;
        if width > MAX_RASTER_EDGE
            || height > MAX_RASTER_EDGE
            || width > u16::MAX as u32
            || height > u16::MAX as u32
            || u64::from(width) * u64::from(height) > limits.max_source_pixels_per_page
        {
            return Err(PdfError::RasterTooLarge {
                page: index + 1,
                width,
                height,
            });
        }

        let pixmap = hayro::render(
            page,
            &cache,
            &interpreter_settings,
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                width: Some(width as u16),
                height: Some(height as u16),
                bg_color: WHITE,
            },
        );
        let rgba: Vec<u8> = bytemuck::cast_vec(pixmap.take_unpremultiplied());
        if rgba.len() != (width * height * 4) as usize {
            return Err(PdfError::InvalidRender);
        }
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pixel in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        optimizer.add_page(&RasterPage::new(width, height, rgb)?)?;
    }

    let output = optimizer.finish()?;
    let output_pages = output.len();
    let bytes = encode_raster_pdf(&output, options.dpi);
    Ok(OptimizedPdf {
        bytes,
        source_pages,
        output_pages,
    })
}

/// Return the number of pages without rendering the document.
pub fn page_count(input: &[u8]) -> Result<usize, PdfError> {
    let pdf = SourcePdf::new(input.to_vec()).map_err(|_| PdfError::Parse)?;
    let count = pdf.pages().len();
    if count == 0 {
        Err(PdfError::Empty)
    } else {
        Ok(count)
    }
}

fn encode_raster_pdf(pages: &[RasterPage], dpi: u32) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let page_ids: Vec<Ref> = (0..pages.len())
        .map(|index| Ref::new(3 + index as i32 * 3))
        .collect();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);

    for (index, page) in pages.iter().enumerate() {
        let page_id = page_ids[index];
        let image_id = Ref::new(page_id.get() + 1);
        let content_id = Ref::new(page_id.get() + 2);
        let image_name = Name(b"PageImage");
        let width_points = page.width as f32 * 72.0 / dpi as f32;
        let height_points = page.height as f32 * 72.0 / dpi as f32;

        let mut output_page = pdf.page(page_id);
        output_page.parent(page_tree_id);
        output_page.media_box(Rect::new(0.0, 0.0, width_points, height_points));
        output_page.contents(content_id);
        output_page
            .resources()
            .x_objects()
            .pair(image_name, image_id);
        output_page.finish();

        let compressed = compress_to_vec_zlib(&page.pixels, CompressionLevel::DefaultLevel as u8);
        let mut image = pdf.image_xobject(image_id, &compressed);
        image.filter(Filter::FlateDecode);
        image.width(page.width as i32);
        image.height(page.height as i32);
        image.color_space().device_rgb();
        image.bits_per_component(8);
        image.finish();

        let mut content = Content::new();
        content.save_state();
        content.transform([width_points, 0.0, 0.0, height_points, 0.0, 0.0]);
        content.x_object(image_name);
        content.restore_state();
        pdf.stream(content_id, &content.finish());
    }

    pdf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_parseable_multi_page_pdf() {
        let pages = vec![
            RasterPage::white(200, 300).unwrap(),
            RasterPage::white(200, 300).unwrap(),
        ];
        let bytes = encode_raster_pdf(&pages, 150);
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(page_count(&bytes).unwrap(), 2);
    }

    #[test]
    fn preserves_top_to_bottom_raster_orientation() {
        let mut source = RasterPage::white(20, 20).unwrap();
        for y in 0..20 {
            let color = if y < 10 { [220, 0, 0] } else { [0, 0, 220] };
            for x in 0..20 {
                let offset = ((y * source.width + x) * 3) as usize;
                source.pixels[offset..offset + 3].copy_from_slice(&color);
            }
        }
        let bytes = encode_raster_pdf(&[source], 72);
        let pdf = SourcePdf::new(bytes).unwrap();
        let page = &pdf.pages()[0];
        let pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings {
                width: Some(20),
                height: Some(20),
                bg_color: WHITE,
                ..Default::default()
            },
        );
        let rgba: Vec<u8> = bytemuck::cast_vec(pixmap.take_unpremultiplied());
        let top = &rgba[((2 * 20 + 10) * 4)..][..3];
        let bottom = &rgba[((17 * 20 + 10) * 4)..][..3];
        assert!(top[0] > top[2], "top half was vertically flipped");
        assert!(bottom[2] > bottom[0], "bottom half was vertically flipped");
    }

    #[test]
    fn round_trips_through_the_complete_pipeline() {
        let mut source = RasterPage::white(300, 400).unwrap();
        for y in 40..360 {
            for x in 30..270 {
                if y % 30 < 8 && x % 32 < 22 {
                    let offset = ((y * source.width + x) * 3) as usize;
                    source.pixels[offset..offset + 3].fill(0);
                }
            }
        }
        let input = encode_raster_pdf(&[source], 144);
        let result = optimize_pdf(
            &input,
            OptimizationOptions {
                width: 300,
                height: 400,
                source_dpi: 72,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.source_pages, 1);
        assert!(result.output_pages >= 1);
        assert_eq!(page_count(&result.bytes).unwrap(), result.output_pages);
    }
}
