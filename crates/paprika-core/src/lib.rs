//! Target-independent raster analysis and layout for Paprika.
//!
//! The engine treats each source page as pixels. It finds content, reading
//! regions, columns, rows, and graphical words, then packs those regions into
//! pages sized for a small screen. No OCR or source-PDF internals are required.

use image::{ImageBuffer, Rgb, RgbImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A packed, top-left-origin RGB page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterPage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RasterPage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, OptimizeError> {
        let expected = pixel_bytes(width, height)?;
        if pixels.len() != expected {
            return Err(OptimizeError::InvalidRaster {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn white(width: u32, height: u32) -> Result<Self, OptimizeError> {
        Ok(Self {
            width,
            height,
            pixels: vec![255; pixel_bytes(width, height)?],
        })
    }

    fn luminance(&self, x: u32, y: u32) -> u8 {
        let offset = ((y * self.width + x) * 3) as usize;
        let r = self.pixels[offset] as u32;
        let g = self.pixels[offset + 1] as u32;
        let b = self.pixels[offset + 2] as u32;
        ((r * 54 + g * 183 + b * 19) >> 8) as u8
    }
}

/// Output layout strategy.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Detect columns and wrap graphical words into new lines.
    #[default]
    Reflow,
    /// Trim each source page, fit it to the output width, and slice vertically.
    FitWidth,
    /// Trim and fit one complete source page into one output page.
    FitPage,
}

/// Shared native and browser optimization settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OptimizationOptions {
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub source_dpi: u32,
    pub margin: u32,
    pub font_size: f32,
    pub threshold: u8,
    pub columns: u8,
}

impl Default for OptimizationOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Reflow,
            width: 758,
            height: 1_024,
            dpi: 167,
            source_dpi: 144,
            margin: 24,
            font_size: 12.0,
            threshold: 245,
            columns: 2,
        }
    }
}

impl OptimizationOptions {
    pub fn validate(&self) -> Result<(), OptimizeError> {
        if !(128..=8_192).contains(&self.width) || !(128..=8_192).contains(&self.height) {
            return Err(OptimizeError::InvalidOptions(
                "output width and height must be between 128 and 8192 pixels".into(),
            ));
        }
        if self.margin.saturating_mul(2) >= self.width
            || self.margin.saturating_mul(2) >= self.height
        {
            return Err(OptimizeError::InvalidOptions(
                "margin leaves no usable output area".into(),
            ));
        }
        if !(36..=600).contains(&self.dpi) || !(36..=600).contains(&self.source_dpi) {
            return Err(OptimizeError::InvalidOptions(
                "DPI must be between 36 and 600".into(),
            ));
        }
        if !(4.0..=72.0).contains(&self.font_size) {
            return Err(OptimizeError::InvalidOptions(
                "font size must be between 4 and 72 points".into(),
            ));
        }
        if !(1..=2).contains(&self.columns) {
            return Err(OptimizeError::InvalidOptions(
                "columns must be 1 or 2".into(),
            ));
        }
        pixel_bytes(self.width, self.height)?;
        Ok(())
    }

    fn content_width(&self) -> u32 {
        self.width - self.margin * 2
    }

    fn content_height(&self) -> u32 {
        self.height - self.margin * 2
    }

    fn target_line_height(&self) -> u32 {
        ((self.font_size * self.dpi as f32 / 72.0).round() as u32).max(6)
    }
}

#[derive(Debug, Error)]
pub enum OptimizeError {
    #[error("invalid raster: expected {expected} RGB bytes, got {actual}")]
    InvalidRaster { expected: usize, actual: usize },
    #[error("invalid options: {0}")]
    InvalidOptions(String),
    #[error("image dimensions are too large")]
    DimensionsTooLarge,
    #[error("failed to construct an image buffer")]
    ImageBuffer,
    #[error("blit rectangle exceeds raster bounds")]
    BlitOutOfBounds,
    #[error(
        "output exceeds the configured memory budget; reduce page dimensions or process fewer pages"
    )]
    OutputLimit,
}

/// Incrementally optimizes source pages while retaining only buffered output pages.
///
/// A caller can render and submit one PDF page at a time, then drain completed
/// output pages for compression. This avoids retaining source rasters or every
/// uncompressed output page for the full document.
pub struct DocumentOptimizer {
    options: OptimizationOptions,
    composer: Composer,
}

impl DocumentOptimizer {
    pub fn new(options: OptimizationOptions) -> Result<Self, OptimizeError> {
        Self::new_with_output_pixel_limit(options, None)
    }

    /// Construct an optimizer with a hard cap on retained output pixels.
    ///
    /// The final PDF encoder currently needs all completed output rasters. A
    /// caller such as the browser adapter should set this to a conservative
    /// budget so an adversarial document fails cleanly instead of exhausting
    /// the process or tab.
    pub fn new_with_output_pixel_limit(
        options: OptimizationOptions,
        max_output_pixels: Option<u64>,
    ) -> Result<Self, OptimizeError> {
        options.validate()?;
        let pixels_per_page = u64::from(options.width) * u64::from(options.height);
        let max_pages = max_output_pixels.map(|pixels| (pixels / pixels_per_page) as usize);
        if max_pages == Some(0) {
            return Err(OptimizeError::OutputLimit);
        }
        let composer = Composer::new(options.clone(), max_pages)?;
        Ok(Self { options, composer })
    }

    pub fn add_page(&mut self, page: &RasterPage) -> Result<(), OptimizeError> {
        let expected = pixel_bytes(page.width, page.height)?;
        if page.pixels.len() != expected {
            return Err(OptimizeError::InvalidRaster {
                expected,
                actual: page.pixels.len(),
            });
        }

        match self.options.mode {
            Mode::Reflow => self.add_reflow_page(page),
            Mode::FitWidth => self.add_fit_width_page(page),
            Mode::FitPage => self.add_fit_page(page),
        }
    }

    /// Remove completed output pages while preserving the current partial page.
    ///
    /// Long-running adapters should call this after each source page and encode
    /// the returned pages before processing more input.
    pub fn take_completed_pages(&mut self) -> Vec<RasterPage> {
        self.composer.take_completed_pages()
    }

    pub fn finish(self) -> Result<Vec<RasterPage>, OptimizeError> {
        self.composer.finish()
    }

    fn add_reflow_page(&mut self, page: &RasterPage) -> Result<(), OptimizeError> {
        let Some(content) = content_bounds(page, self.options.threshold) else {
            self.composer.paragraph_break()?;
            return Ok(());
        };

        // Establish page-level columns first. Processing horizontal regions
        // first interleaves two-column pages as top-left, top-right,
        // bottom-left, bottom-right instead of completing the left column.
        for column in detect_columns(page, content, self.options.threshold, self.options.columns) {
            for region in vertical_regions(page, column, self.options.threshold) {
                let rows = ink_runs_y(page, region, self.options.threshold);
                if rows.is_empty() {
                    continue;
                }
                let heights: Vec<u32> = rows.iter().map(|row| row.height).collect();
                let median_height = median(&heights).max(1);
                let max_text_height = (self.options.source_dpi / 3).max(24);
                let mut previous: Option<Rect> = None;

                for row in rows {
                    let paragraph = previous.is_some_and(|prev| {
                        let gap = row.y.saturating_sub(prev.bottom());
                        let indent = row.x.abs_diff(prev.x);
                        gap > median_height || indent > median_height * 2
                    });
                    if paragraph {
                        self.composer.paragraph_break()?;
                    }

                    // A lone photograph or connected scan region has itself as
                    // the median row. The absolute source-DPI guard prevents it
                    // from being flattened to one target text line.
                    if row.height > median_height.saturating_mul(5) / 2
                        || row.height > max_text_height
                    {
                        self.composer.place_block(page, row)?;
                    } else {
                        let words = word_rects(page, row, self.options.threshold);
                        if words.is_empty() {
                            self.composer.place_block(page, row)?;
                        } else {
                            for word in words {
                                self.composer.place_word(page, word, row.height)?;
                            }
                        }
                    }
                    previous = Some(row);
                }
                self.composer.paragraph_break()?;
            }
            self.composer.paragraph_break()?;
        }
        self.composer.paragraph_break()
    }

    fn add_fit_width_page(&mut self, page: &RasterPage) -> Result<(), OptimizeError> {
        let Some(content) = content_bounds(page, self.options.threshold) else {
            return self.composer.blank_page();
        };
        self.composer.fit_width(page, content)
    }

    fn add_fit_page(&mut self, page: &RasterPage) -> Result<(), OptimizeError> {
        let content = content_bounds(page, self.options.threshold).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: page.width,
            height: page.height,
        });
        self.composer.fit_page(page, content)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn right(self) -> u32 {
        self.x + self.width
    }

    fn bottom(self) -> u32 {
        self.y + self.height
    }
}

struct Composer {
    options: OptimizationOptions,
    pages: Vec<RasterPage>,
    current: RasterPage,
    cursor_x: u32,
    cursor_y: u32,
    line_height: u32,
    dirty: bool,
    emitted_pages: usize,
    max_pages: Option<usize>,
}

impl Composer {
    fn new(options: OptimizationOptions, max_pages: Option<usize>) -> Result<Self, OptimizeError> {
        let current = RasterPage::white(options.width, options.height)?;
        let margin = options.margin;
        Ok(Self {
            options,
            pages: Vec::new(),
            current,
            cursor_x: margin,
            cursor_y: margin,
            line_height: 0,
            dirty: false,
            emitted_pages: 0,
            max_pages,
        })
    }

    fn take_completed_pages(&mut self) -> Vec<RasterPage> {
        self.emitted_pages += self.pages.len();
        std::mem::take(&mut self.pages)
    }

    fn finish(mut self) -> Result<Vec<RasterPage>, OptimizeError> {
        if self.dirty || (self.pages.is_empty() && self.emitted_pages == 0) {
            // Moving the active canvas into the result does not retain another
            // raster, so it cannot increase the pixel working set.
            self.pages.push(self.current);
        }
        Ok(self.pages)
    }

    fn ensure_replacement_capacity(&self) -> Result<(), OptimizeError> {
        // The active canvas is retained alongside every completed page. A
        // replacement therefore needs one more slot than pages.len().
        if self
            .max_pages
            .is_some_and(|limit| self.pages.len().saturating_add(1) >= limit)
        {
            Err(OptimizeError::OutputLimit)
        } else {
            Ok(())
        }
    }

    fn fresh_page(&mut self) -> Result<(), OptimizeError> {
        if self.dirty {
            self.ensure_replacement_capacity()?;
            let replacement = RasterPage::white(self.options.width, self.options.height)?;
            self.pages
                .push(std::mem::replace(&mut self.current, replacement));
        }
        self.cursor_x = self.options.margin;
        self.cursor_y = self.options.margin;
        self.line_height = 0;
        self.dirty = false;
        Ok(())
    }

    fn blank_page(&mut self) -> Result<(), OptimizeError> {
        self.fresh_page()?;
        self.dirty = true;
        self.fresh_page()
    }

    fn new_line(&mut self) -> Result<(), OptimizeError> {
        if self.line_height == 0 {
            return Ok(());
        }
        let gap = (self.options.target_line_height() / 5).max(2);
        self.cursor_y = self.cursor_y.saturating_add(self.line_height + gap);
        self.cursor_x = self.options.margin;
        self.line_height = 0;
        if self.cursor_y >= self.options.height - self.options.margin {
            self.fresh_page()?;
        }
        Ok(())
    }

    fn paragraph_break(&mut self) -> Result<(), OptimizeError> {
        self.new_line()?;
        if self.dirty && self.cursor_y > self.options.margin {
            self.cursor_y = self
                .cursor_y
                .saturating_add((self.options.target_line_height() / 2).max(3));
            if self.cursor_y >= self.options.height - self.options.margin {
                self.fresh_page()?;
            }
        }
        Ok(())
    }

    fn place_word(
        &mut self,
        source: &RasterPage,
        source_rect: Rect,
        source_line_height: u32,
    ) -> Result<(), OptimizeError> {
        let target_height = self.options.target_line_height();
        let scale = target_height as f32 / source_line_height.max(1) as f32;
        let mut width = ((source_rect.width as f32 * scale).round() as u32).max(1);
        let mut height = ((source_rect.height as f32 * scale).round() as u32).max(1);
        let available_width = self.options.content_width();
        let available_height = self.options.content_height();
        if width > available_width || height > available_height {
            let fit = (available_width as f32 / width as f32)
                .min(available_height as f32 / height as f32);
            width = ((width as f32 * fit).round() as u32)
                .max(1)
                .min(available_width);
            height = ((height as f32 * fit).round() as u32)
                .max(1)
                .min(available_height);
        }

        let gap = (target_height / 4).max(2);
        if self.cursor_x > self.options.margin
            && self.cursor_x.saturating_add(width) > self.options.width - self.options.margin
        {
            self.new_line()?;
        }
        if self.cursor_y.saturating_add(height) > self.options.height - self.options.margin {
            self.fresh_page()?;
        }

        blit_scaled(
            source,
            source_rect,
            &mut self.current,
            self.cursor_x,
            self.cursor_y,
            width,
            height,
        )?;
        self.cursor_x = self.cursor_x.saturating_add(width + gap);
        self.line_height = self.line_height.max(height);
        self.dirty = true;
        Ok(())
    }

    fn place_block(&mut self, source: &RasterPage, source_rect: Rect) -> Result<(), OptimizeError> {
        self.new_line()?;
        let width_scale = self.options.content_width() as f32 / source_rect.width.max(1) as f32;
        let height_scale = self.options.content_height() as f32 / source_rect.height.max(1) as f32;
        let scale = width_scale.min(height_scale).min(1.5);
        let width = ((source_rect.width as f32 * scale).round() as u32).max(1);
        let height = ((source_rect.height as f32 * scale).round() as u32).max(1);
        if self.cursor_y.saturating_add(height) > self.options.height - self.options.margin {
            self.fresh_page()?;
        }
        let x = self.options.margin + (self.options.content_width() - width) / 2;
        blit_scaled(
            source,
            source_rect,
            &mut self.current,
            x,
            self.cursor_y,
            width,
            height,
        )?;
        self.cursor_y = self
            .cursor_y
            .saturating_add(height + self.options.target_line_height() / 2);
        self.cursor_x = self.options.margin;
        self.dirty = true;
        Ok(())
    }

    fn fit_width(&mut self, source: &RasterPage, source_rect: Rect) -> Result<(), OptimizeError> {
        self.paragraph_break()?;
        let scale = self.options.content_width() as f32 / source_rect.width.max(1) as f32;
        let source_chunk_height = (self.options.content_height() as f32 / scale)
            .floor()
            .max(1.0) as u32;
        let mut offset = 0;
        while offset < source_rect.height {
            if self.dirty {
                self.fresh_page()?;
            }
            let chunk_height = source_chunk_height.min(source_rect.height - offset);
            let output_height = ((chunk_height as f32 * scale).round() as u32)
                .min(self.options.content_height())
                .max(1);
            blit_scaled(
                source,
                Rect {
                    x: source_rect.x,
                    y: source_rect.y + offset,
                    width: source_rect.width,
                    height: chunk_height,
                },
                &mut self.current,
                self.options.margin,
                self.options.margin,
                self.options.content_width(),
                output_height,
            )?;
            self.dirty = true;
            offset += chunk_height;
            if offset < source_rect.height {
                self.fresh_page()?;
            }
        }
        self.fresh_page()
    }

    fn fit_page(&mut self, source: &RasterPage, source_rect: Rect) -> Result<(), OptimizeError> {
        if self.dirty {
            self.fresh_page()?;
        }
        let scale = (self.options.content_width() as f32 / source_rect.width.max(1) as f32)
            .min(self.options.content_height() as f32 / source_rect.height.max(1) as f32);
        let width = ((source_rect.width as f32 * scale).round() as u32).max(1);
        let height = ((source_rect.height as f32 * scale).round() as u32).max(1);
        let x = self.options.margin + (self.options.content_width() - width) / 2;
        let y = self.options.margin + (self.options.content_height() - height) / 2;
        blit_scaled(source, source_rect, &mut self.current, x, y, width, height)?;
        self.dirty = true;
        self.fresh_page()
    }
}

fn pixel_bytes(width: u32, height: u32) -> Result<usize, OptimizeError> {
    if width == 0 || height == 0 {
        return Err(OptimizeError::InvalidOptions(
            "image width and height must be greater than zero".into(),
        ));
    }
    let pixels = width
        .checked_mul(height)
        .and_then(|count| count.checked_mul(3))
        .ok_or(OptimizeError::DimensionsTooLarge)?;
    usize::try_from(pixels).map_err(|_| OptimizeError::DimensionsTooLarge)
}

fn is_ink(page: &RasterPage, x: u32, y: u32, threshold: u8) -> bool {
    page.luminance(x, y) < threshold
}

fn content_bounds(page: &RasterPage, threshold: u8) -> Option<Rect> {
    let mut min_x = page.width;
    let mut min_y = page.height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for y in 0..page.height {
        for x in 0..page.width {
            if is_ink(page, x, y, threshold) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    if !found {
        return None;
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn vertical_regions(page: &RasterPage, bounds: Rect, threshold: u8) -> Vec<Rect> {
    let mut occupied = vec![false; bounds.height as usize];
    for local_y in 0..bounds.height {
        let y = bounds.y + local_y;
        occupied[local_y as usize] =
            (bounds.x..bounds.right()).any(|x| is_ink(page, x, y, threshold));
    }
    let min_separator = (bounds.height / 100).max(8);
    let mut regions = Vec::new();
    let mut start = 0;
    let mut y = 0;
    while y < bounds.height {
        if occupied[y as usize] {
            y += 1;
            continue;
        }
        let gap_start = y;
        while y < bounds.height && !occupied[y as usize] {
            y += 1;
        }
        if y - gap_start >= min_separator && gap_start > start {
            regions.push(Rect {
                x: bounds.x,
                y: bounds.y + start,
                width: bounds.width,
                height: gap_start - start,
            });
            start = y;
        }
    }
    if start < bounds.height {
        regions.push(Rect {
            x: bounds.x,
            y: bounds.y + start,
            width: bounds.width,
            height: bounds.height - start,
        });
    }
    if regions.is_empty() {
        vec![bounds]
    } else {
        regions
    }
}

fn detect_columns(page: &RasterPage, region: Rect, threshold: u8, max_columns: u8) -> Vec<Rect> {
    if max_columns < 2 || region.width < 128 || region.height < 48 {
        return vec![region];
    }
    let max_ink = (region.height / 100).max(1);
    let min_gutter = (region.width / 50).max(6);
    let mut candidates = Vec::new();
    let mut x = 0;
    while x < region.width {
        let ink = (region.y..region.bottom())
            .filter(|&y| is_ink(page, region.x + x, y, threshold))
            .count() as u32;
        if ink > max_ink {
            x += 1;
            continue;
        }
        let start = x;
        while x < region.width {
            let ink = (region.y..region.bottom())
                .filter(|&y| is_ink(page, region.x + x, y, threshold))
                .count() as u32;
            if ink > max_ink {
                break;
            }
            x += 1;
        }
        if x - start >= min_gutter {
            let midpoint = start + (x - start) / 2;
            let left = midpoint;
            let right = region.width - midpoint;
            if left >= region.width / 4 && right >= region.width / 4 {
                let center_distance = midpoint.abs_diff(region.width / 2);
                candidates.push((x - start, center_distance, midpoint));
            }
        }
    }
    let Some((_, _, split)) = candidates
        .into_iter()
        .max_by_key(|(width, distance, _)| (*width, std::cmp::Reverse(*distance)))
    else {
        return vec![region];
    };

    vec![
        Rect {
            x: region.x,
            y: region.y,
            width: split,
            height: region.height,
        },
        Rect {
            x: region.x + split,
            y: region.y,
            width: region.width - split,
            height: region.height,
        },
    ]
}

fn ink_runs_y(page: &RasterPage, region: Rect, threshold: u8) -> Vec<Rect> {
    let mut rows = Vec::new();
    let mut y = region.y;
    while y < region.bottom() {
        let occupied = (region.x..region.right()).any(|x| is_ink(page, x, y, threshold));
        if !occupied {
            y += 1;
            continue;
        }
        let start = y;
        y += 1;
        while y < region.bottom()
            && (region.x..region.right()).any(|x| is_ink(page, x, y, threshold))
        {
            y += 1;
        }
        let mut min_x = region.right();
        let mut max_x = region.x;
        for row_y in start..y {
            for x in region.x..region.right() {
                if is_ink(page, x, row_y, threshold) {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
        }
        if min_x <= max_x {
            rows.push(Rect {
                x: min_x,
                y: start,
                width: max_x - min_x + 1,
                height: y - start,
            });
        }
    }
    rows
}

fn word_rects(page: &RasterPage, row: Rect, threshold: u8) -> Vec<Rect> {
    let mut glyphs = Vec::new();
    let mut x = row.x;
    while x < row.right() {
        let occupied = (row.y..row.bottom()).any(|y| is_ink(page, x, y, threshold));
        if !occupied {
            x += 1;
            continue;
        }
        let start = x;
        x += 1;
        while x < row.right() && (row.y..row.bottom()).any(|y| is_ink(page, x, y, threshold)) {
            x += 1;
        }
        glyphs.push((start, x));
    }
    if glyphs.is_empty() {
        return Vec::new();
    }

    let join_gap = (row.height / 5).max(1);
    let mut words = Vec::new();
    let mut start = glyphs[0].0;
    let mut end = glyphs[0].1;
    for &(next_start, next_end) in glyphs.iter().skip(1) {
        if next_start - end <= join_gap {
            end = next_end;
        } else {
            words.push(tight_rect(
                page,
                Rect {
                    x: start,
                    y: row.y,
                    width: end - start,
                    height: row.height,
                },
                threshold,
            ));
            start = next_start;
            end = next_end;
        }
    }
    words.push(tight_rect(
        page,
        Rect {
            x: start,
            y: row.y,
            width: end - start,
            height: row.height,
        },
        threshold,
    ));
    words
}

fn tight_rect(page: &RasterPage, rect: Rect, threshold: u8) -> Rect {
    let mut min_y = rect.bottom();
    let mut max_y = rect.y;
    for y in rect.y..rect.bottom() {
        if (rect.x..rect.right()).any(|x| is_ink(page, x, y, threshold)) {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    Rect {
        x: rect.x,
        y: min_y,
        width: rect.width,
        height: max_y.saturating_sub(min_y) + 1,
    }
}

fn median(values: &[u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn blit_scaled(
    source: &RasterPage,
    source_rect: Rect,
    destination: &mut RasterPage,
    destination_x: u32,
    destination_y: u32,
    destination_width: u32,
    destination_height: u32,
) -> Result<(), OptimizeError> {
    let source_in_bounds = source_rect.width > 0
        && source_rect.height > 0
        && source_rect
            .x
            .checked_add(source_rect.width)
            .is_some_and(|right| right <= source.width)
        && source_rect
            .y
            .checked_add(source_rect.height)
            .is_some_and(|bottom| bottom <= source.height);
    let destination_in_bounds = destination_width > 0
        && destination_height > 0
        && destination_x
            .checked_add(destination_width)
            .is_some_and(|right| right <= destination.width)
        && destination_y
            .checked_add(destination_height)
            .is_some_and(|bottom| bottom <= destination.height);
    if !source_in_bounds || !destination_in_bounds {
        return Err(OptimizeError::BlitOutOfBounds);
    }

    let mut crop = Vec::with_capacity(pixel_bytes(source_rect.width, source_rect.height)?);
    for y in source_rect.y..source_rect.bottom() {
        let start = ((y * source.width + source_rect.x) * 3) as usize;
        let end = start + (source_rect.width * 3) as usize;
        crop.extend_from_slice(&source.pixels[start..end]);
    }
    let image: RgbImage =
        ImageBuffer::<Rgb<u8>, _>::from_raw(source_rect.width, source_rect.height, crop)
            .ok_or(OptimizeError::ImageBuffer)?;
    let resized = image::imageops::resize(
        &image,
        destination_width,
        destination_height,
        FilterType::Triangle,
    );
    for y in 0..destination_height {
        let source_start = (y * destination_width * 3) as usize;
        let source_end = source_start + (destination_width * 3) as usize;
        let destination_start =
            (((destination_y + y) * destination.width + destination_x) * 3) as usize;
        let destination_end = destination_start + (destination_width * 3) as usize;
        destination.pixels[destination_start..destination_end]
            .copy_from_slice(&resized.as_raw()[source_start..source_end]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: u32, height: u32, rects: &[Rect]) -> RasterPage {
        let mut page = RasterPage::white(width, height).unwrap();
        for rect in rects {
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let offset = ((y * width + x) * 3) as usize;
                    page.pixels[offset..offset + 3].fill(0);
                }
            }
        }
        page
    }

    fn dark_rows(page: &RasterPage) -> Vec<u32> {
        (0..page.height)
            .filter(|&y| (0..page.width).any(|x| page.luminance(x, y) < 128))
            .collect()
    }

    #[test]
    fn rejects_invalid_raster_length() {
        let error = RasterPage::new(10, 10, vec![0; 10]).unwrap_err();
        assert!(matches!(error, OptimizeError::InvalidRaster { .. }));
    }

    #[test]
    fn rejects_zero_sized_rasters() {
        assert!(RasterPage::new(0, 10, Vec::new()).is_err());
        assert!(RasterPage::white(10, 0).is_err());
    }

    #[test]
    fn blank_pages_are_safe_in_every_mode() {
        let source = RasterPage::white(100, 100).unwrap();
        for mode in [Mode::Reflow, Mode::FitWidth, Mode::FitPage] {
            let mut optimizer = DocumentOptimizer::new(OptimizationOptions {
                mode,
                width: 128,
                height: 128,
                margin: 8,
                ..Default::default()
            })
            .unwrap();
            optimizer.add_page(&source).unwrap();
            let output = optimizer.finish().unwrap();
            assert_eq!(output.len(), 1);
            assert!(output[0].pixels.iter().all(|&channel| channel == 255));
        }
    }

    #[test]
    fn trims_content_bounds() {
        let source = page(
            100,
            80,
            &[Rect {
                x: 20,
                y: 15,
                width: 30,
                height: 10,
            }],
        );
        assert_eq!(
            content_bounds(&source, 245),
            Some(Rect {
                x: 20,
                y: 15,
                width: 30,
                height: 10,
            })
        );
    }

    #[test]
    fn detects_two_columns_from_sustained_gutter() {
        let source = page(
            240,
            180,
            &[
                Rect {
                    x: 10,
                    y: 10,
                    width: 80,
                    height: 150,
                },
                Rect {
                    x: 150,
                    y: 10,
                    width: 80,
                    height: 150,
                },
            ],
        );
        let columns = detect_columns(
            &source,
            Rect {
                x: 10,
                y: 10,
                width: 220,
                height: 150,
            },
            245,
            2,
        );
        assert_eq!(columns.len(), 2);
        assert!(columns[0].right() <= columns[1].x);
    }

    #[test]
    fn reads_the_complete_left_column_before_the_right() {
        let mut source = RasterPage::white(240, 180).unwrap();
        for (rect, color) in [
            (
                Rect {
                    x: 10,
                    y: 10,
                    width: 80,
                    height: 10,
                },
                [180, 0, 0],
            ),
            (
                Rect {
                    x: 10,
                    y: 120,
                    width: 80,
                    height: 10,
                },
                [180, 0, 0],
            ),
            (
                Rect {
                    x: 150,
                    y: 10,
                    width: 80,
                    height: 10,
                },
                [0, 0, 180],
            ),
            (
                Rect {
                    x: 150,
                    y: 120,
                    width: 80,
                    height: 10,
                },
                [0, 0, 180],
            ),
        ] {
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let offset = ((y * source.width + x) * 3) as usize;
                    source.pixels[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }
        let mut optimizer = DocumentOptimizer::new(OptimizationOptions {
            width: 128,
            height: 300,
            margin: 8,
            dpi: 144,
            source_dpi: 144,
            font_size: 8.0,
            ..Default::default()
        })
        .unwrap();
        optimizer.add_page(&source).unwrap();
        let output = optimizer.finish().unwrap();
        let red_y = output[0]
            .pixels
            .chunks_exact(3)
            .enumerate()
            .filter(|(_, pixel)| u16::from(pixel[0]) > u16::from(pixel[2]) + 40)
            .map(|(index, _)| index as u32 / output[0].width)
            .max()
            .unwrap();
        let blue_y = output[0]
            .pixels
            .chunks_exact(3)
            .enumerate()
            .filter(|(_, pixel)| u16::from(pixel[2]) > u16::from(pixel[0]) + 40)
            .map(|(index, _)| index as u32 / output[0].width)
            .min()
            .unwrap();
        assert!(
            red_y < blue_y,
            "right column appeared before left was complete"
        );
    }

    #[test]
    fn reflow_wraps_graphical_words() {
        let mut rects = Vec::new();
        for x in (10..190).step_by(18) {
            rects.push(Rect {
                x,
                y: 20,
                width: 12,
                height: 12,
            });
        }
        let source = page(200, 60, &rects);
        let options = OptimizationOptions {
            width: 128,
            height: 180,
            margin: 8,
            dpi: 144,
            source_dpi: 144,
            font_size: 12.0,
            columns: 1,
            ..Default::default()
        };
        let mut optimizer = DocumentOptimizer::new(options).unwrap();
        optimizer.add_page(&source).unwrap();
        let output = optimizer.finish().unwrap();
        let rows = dark_rows(&output[0]);
        assert!(!rows.is_empty());
        assert!(rows.last().unwrap() - rows.first().unwrap() > 20);
    }

    #[test]
    fn a_lone_image_remains_a_block() {
        let source = page(
            120,
            120,
            &[Rect {
                x: 20,
                y: 20,
                width: 80,
                height: 80,
            }],
        );
        let mut optimizer = DocumentOptimizer::new(OptimizationOptions {
            width: 160,
            height: 200,
            margin: 10,
            source_dpi: 144,
            ..Default::default()
        })
        .unwrap();
        optimizer.add_page(&source).unwrap();
        let output = optimizer.finish().unwrap();
        let rows = dark_rows(&output[0]);
        assert!(rows.last().unwrap() - rows.first().unwrap() > 70);
    }

    #[test]
    fn completed_pages_can_be_drained_without_a_trailing_blank_page() {
        let source = page(
            100,
            100,
            &[Rect {
                x: 10,
                y: 10,
                width: 80,
                height: 80,
            }],
        );
        let mut optimizer = DocumentOptimizer::new(OptimizationOptions {
            mode: Mode::FitPage,
            width: 128,
            height: 128,
            margin: 8,
            ..Default::default()
        })
        .unwrap();

        optimizer.add_page(&source).unwrap();
        assert_eq!(optimizer.take_completed_pages().len(), 1);
        assert!(optimizer.take_completed_pages().is_empty());
        assert!(optimizer.finish().unwrap().is_empty());
    }

    #[test]
    fn output_pixel_budget_counts_the_active_canvas() {
        let source = page(
            100,
            100,
            &[Rect {
                x: 10,
                y: 10,
                width: 80,
                height: 80,
            }],
        );
        let options = OptimizationOptions {
            mode: Mode::FitPage,
            width: 128,
            height: 128,
            margin: 8,
            ..Default::default()
        };
        let mut optimizer =
            DocumentOptimizer::new_with_output_pixel_limit(options, Some(2 * 128_u64 * 128))
                .unwrap();
        optimizer.add_page(&source).unwrap();
        assert!(matches!(
            optimizer.add_page(&source),
            Err(OptimizeError::OutputLimit)
        ));
    }

    #[test]
    fn draining_completed_pages_releases_pixel_budget() {
        let source = page(
            100,
            100,
            &[Rect {
                x: 10,
                y: 10,
                width: 80,
                height: 80,
            }],
        );
        let options = OptimizationOptions {
            mode: Mode::FitPage,
            width: 128,
            height: 128,
            margin: 8,
            ..Default::default()
        };
        let mut optimizer =
            DocumentOptimizer::new_with_output_pixel_limit(options, Some(2 * 128_u64 * 128))
                .unwrap();

        optimizer.add_page(&source).unwrap();
        assert_eq!(optimizer.take_completed_pages().len(), 1);
        optimizer.add_page(&source).unwrap();
    }

    #[test]
    fn oversized_words_fit_inside_both_canvas_dimensions() {
        let source = page(
            100,
            200,
            &[Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 200,
            }],
        );
        let options = OptimizationOptions {
            width: 128,
            height: 128,
            margin: 8,
            dpi: 600,
            font_size: 72.0,
            ..Default::default()
        };
        options.validate().unwrap();
        let mut composer = Composer::new(options, None).unwrap();

        composer
            .place_word(
                &source,
                Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 200,
                },
                200,
            )
            .unwrap();

        let rows = dark_rows(&composer.current);
        assert_eq!(rows.first(), Some(&8));
        assert_eq!(rows.last(), Some(&119));
        assert!(
            (120..composer.current.height)
                .all(|y| (0..composer.current.width)
                    .all(|x| composer.current.luminance(x, y) == 255))
        );
    }

    #[test]
    fn out_of_bounds_blits_fail_without_modifying_the_destination() {
        let source = page(
            4,
            4,
            &[Rect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            }],
        );
        let mut destination = RasterPage::white(4, 4).unwrap();
        let original = destination.clone();

        let error = blit_scaled(
            &source,
            Rect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            &mut destination,
            1,
            0,
            4,
            4,
        )
        .unwrap_err();

        assert!(matches!(error, OptimizeError::BlitOutOfBounds));
        assert_eq!(destination, original);
    }

    #[test]
    fn fit_page_preserves_requested_canvas() {
        let source = page(
            300,
            500,
            &[Rect {
                x: 20,
                y: 20,
                width: 260,
                height: 460,
            }],
        );
        let options = OptimizationOptions {
            mode: Mode::FitPage,
            width: 200,
            height: 300,
            margin: 10,
            ..Default::default()
        };
        let mut optimizer = DocumentOptimizer::new(options).unwrap();
        optimizer.add_page(&source).unwrap();
        let output = optimizer.finish().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!((output[0].width, output[0].height), (200, 300));
    }
}
