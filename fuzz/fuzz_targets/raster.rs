#![no_main]

use libfuzzer_sys::fuzz_target;
use paprika_core::{Mode, OptimizationOptions};
use paprika_pdf::{PdfLimits, optimize_pdf_with_limits};

fuzz_target!(|input: &[u8]| {
    if input.len() > 64 * 1024 {
        return;
    }
    let _ = optimize_pdf_with_limits(
        input,
        OptimizationOptions {
            mode: Mode::FitPage,
            width: 128,
            height: 192,
            dpi: 96,
            source_dpi: 72,
            margin: 4,
            ..Default::default()
        },
        PdfLimits {
            max_pages: 32,
            max_source_pixels_per_page: 256 * 1024,
            max_output_pixels: 512 * 1024,
            max_output_bytes: 512 * 1024,
        },
    );
});
