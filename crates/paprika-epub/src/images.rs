use pdf_oxide::geometry::Rect;
use pdf_oxide::{PdfDocument, PdfImageHandle};

use super::buffer::BoundedBuffer;
use super::geometry::overlap_fraction;
use super::model::{ImagePlacement, PageImage};
use super::{
    EpubError, MAX_IMAGE_DECODE_PIXELS, MAX_IMAGE_DECODE_PIXELS_PER_PAGE,
    MAX_IMAGE_DECODE_PIXELS_TOTAL, MAX_IMAGE_OBJECTS_PER_PAGE, MAX_IMAGE_OBJECTS_TOTAL,
    MAX_IMAGE_PIXELS, MIN_IMAGE_PIXELS,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct PageImageCollection<'a> {
    pub(super) index: usize,
    pub(super) excluded_regions: &'a [Rect],
    pub(super) image_only: bool,
    pub(super) bounds: Option<Rect>,
}

#[derive(Debug, Default)]
pub(super) struct ImageDecodeBudget {
    pub(super) objects: usize,
    pub(super) pixels: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ImageRecoveryPlan {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) downsampled: bool,
    pub(super) visual_page: bool,
}

pub(super) fn account_asset(
    total_bytes: &mut usize,
    bytes: usize,
    max_bytes: usize,
) -> Result<(), EpubError> {
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

pub(super) fn account_image_objects(
    budget: &mut ImageDecodeBudget,
    page_index: usize,
    page_objects: usize,
) -> Result<(), EpubError> {
    if page_objects > MAX_IMAGE_OBJECTS_PER_PAGE {
        return Err(EpubError::TooManyImageObjects {
            page: page_index + 1,
            objects: page_objects,
            limit: MAX_IMAGE_OBJECTS_PER_PAGE,
        });
    }
    budget.objects =
        budget
            .objects
            .checked_add(page_objects)
            .ok_or(EpubError::TooManyImageObjectsTotal {
                objects: usize::MAX,
                limit: MAX_IMAGE_OBJECTS_TOTAL,
            })?;
    if budget.objects > MAX_IMAGE_OBJECTS_TOTAL {
        return Err(EpubError::TooManyImageObjectsTotal {
            objects: budget.objects,
            limit: MAX_IMAGE_OBJECTS_TOTAL,
        });
    }
    Ok(())
}

pub(super) fn reserve_image_decode(
    budget: &mut ImageDecodeBudget,
    page_index: usize,
    page_pixels: &mut u64,
    pixels: u64,
) -> Result<(), EpubError> {
    *page_pixels = page_pixels
        .checked_add(pixels)
        .ok_or(EpubError::PageImageDecodeLimit {
            page: page_index + 1,
            limit: MAX_IMAGE_DECODE_PIXELS_PER_PAGE / 1_000_000,
        })?;
    if *page_pixels > MAX_IMAGE_DECODE_PIXELS_PER_PAGE {
        return Err(EpubError::PageImageDecodeLimit {
            page: page_index + 1,
            limit: MAX_IMAGE_DECODE_PIXELS_PER_PAGE / 1_000_000,
        });
    }
    budget.pixels = budget
        .pixels
        .checked_add(pixels)
        .ok_or(EpubError::ImageDecodeLimit {
            limit: MAX_IMAGE_DECODE_PIXELS_TOTAL / 1_000_000,
        })?;
    if budget.pixels > MAX_IMAGE_DECODE_PIXELS_TOTAL {
        return Err(EpubError::ImageDecodeLimit {
            limit: MAX_IMAGE_DECODE_PIXELS_TOTAL / 1_000_000,
        });
    }
    Ok(())
}

pub(super) fn source_page_bounds(document: &PdfDocument, page_index: usize) -> Option<Rect> {
    let (x0, y0, x1, y1) = document.get_page_media_box(page_index).ok()?;
    let left = x0.min(x1);
    let bottom = y0.min(y1);
    let width = (x1 - x0).abs();
    let height = (y1 - y0).abs();
    (width > 0.0 && height > 0.0).then(|| Rect::new(left, bottom, width, height))
}

pub(super) fn image_recovery_plan(
    width: u32,
    height: u32,
    bbox: Rect,
    image_only_page: bool,
    page_bounds: Option<Rect>,
) -> Option<ImageRecoveryPlan> {
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if !(MIN_IMAGE_PIXELS..=MAX_IMAGE_DECODE_PIXELS).contains(&pixels) {
        return None;
    }
    let visual_page =
        image_only_page && page_bounds.is_some_and(|page| overlap_fraction(page, bbox) >= 0.75);
    if pixels > MAX_IMAGE_PIXELS && !visual_page {
        return None;
    }
    let (width, height) = if pixels > MAX_IMAGE_PIXELS {
        downsample_dimensions(width, height, MAX_IMAGE_PIXELS)
    } else {
        (width, height)
    };
    Some(ImageRecoveryPlan {
        width,
        height,
        downsampled: pixels > MAX_IMAGE_PIXELS,
        visual_page,
    })
}

fn downsample_dimensions(width: u32, height: u32, max_pixels: u64) -> (u32, u32) {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels <= max_pixels || pixels == 0 {
        return (width, height);
    }
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let target_width = ((width as f64 * scale).floor() as u32).max(1);
    let mut target_height = ((height as f64 * scale).floor() as u32).max(1);
    while u64::from(target_width) * u64::from(target_height) > max_pixels {
        target_height -= 1;
    }
    (target_width, target_height)
}

pub(super) fn collect_page_images(
    handles: &[PdfImageHandle<'_>],
    context: PageImageCollection<'_>,
    decode_budget: &mut ImageDecodeBudget,
    total_bytes: &mut usize,
    max_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<PageImage>, EpubError> {
    let PageImageCollection {
        index: page_index,
        excluded_regions,
        image_only: image_only_page,
        bounds: page_bounds,
    } = context;
    let mut images = Vec::new();
    let mut page_decode_pixels = 0u64;

    for (image_index, handle) in handles.iter().enumerate() {
        let pixels = u64::from(handle.width).saturating_mul(u64::from(handle.height));
        let covered_by_figure = excluded_regions
            .iter()
            .any(|region| overlap_fraction(handle.bbox, *region) >= 0.5);
        if covered_by_figure || pixels < MIN_IMAGE_PIXELS {
            continue;
        }
        let Some(plan) = image_recovery_plan(
            handle.width,
            handle.height,
            handle.bbox,
            image_only_page,
            page_bounds,
        ) else {
            if pixels > MAX_IMAGE_DECODE_PIXELS {
                warnings.push(format!(
                    "Image {} from page {} exceeds the 32 megapixel decode limit and was skipped.",
                    image_index + 1,
                    page_index + 1
                ));
            } else if pixels > MAX_IMAGE_PIXELS {
                warnings.push(format!(
                    "Image {} from page {} exceeds 4 megapixels and was skipped because it is not a full-page scan.",
                    image_index + 1,
                    page_index + 1
                ));
            }
            continue;
        };
        reserve_image_decode(decode_budget, page_index, &mut page_decode_pixels, pixels)?;
        let image = match handle.decode().and_then(|image| image.to_dynamic_image()) {
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
        let image = if plan.downsampled {
            warnings.push(format!(
                "Full-page scan {} from page {} was downsampled from {}x{} to {}x{} pixels.",
                image_index + 1,
                page_index + 1,
                handle.width,
                handle.height,
                plan.width,
                plan.height
            ));
            image.resize_exact(
                plan.width,
                plan.height,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            image
        };
        let remaining_bytes = max_bytes.saturating_sub(*total_bytes);
        let mut output = BoundedBuffer::new(remaining_bytes);
        let encode_result = image.write_to(&mut output, image::ImageFormat::Png);
        if output.limit_exceeded {
            return Err(EpubError::AssetsTooLarge {
                limit: max_bytes / (1024 * 1024),
            });
        }
        if let Err(error) = encode_result {
            warnings.push(format!(
                "Could not encode image {} from page {}: {error}",
                image_index + 1,
                page_index + 1
            ));
            continue;
        }
        let bytes = output.into_inner();
        account_asset(total_bytes, bytes.len(), max_bytes)?;
        images.push(PageImage {
            href: format!(
                "images/page-{:04}-{:02}.png",
                page_index + 1,
                image_index + 1
            ),
            bytes,
            alt: if plan.visual_page {
                format!("Scan of source page {}", page_index + 1)
            } else {
                format!(
                    "Image {} from source page {}",
                    image_index + 1,
                    page_index + 1
                )
            },
            placement: if plan.visual_page {
                ImagePlacement::VisualPageFallback
            } else {
                ImagePlacement::EndOfPage
            },
        });
    }

    Ok(images)
}
