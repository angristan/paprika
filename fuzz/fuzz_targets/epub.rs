#![no_main]

use libfuzzer_sys::fuzz_target;
use paprika_epub::{EpubOptions, convert_pdf_to_epub};

fuzz_target!(|input: &[u8]| {
    // Keep each iteration cheap enough for continuous fuzzing. Dedicated
    // regression tests exercise visual rendering and larger asset budgets.
    if input.len() > 64 * 1024 {
        return;
    }
    let _ = convert_pdf_to_epub(
        input,
        EpubOptions {
            title: "Fuzz input".to_string(),
            include_images: false,
            max_pages: 8,
            max_asset_bytes: 512 * 1024,
            max_semantic_bytes: 512 * 1024,
            max_output_bytes: 1024 * 1024,
            ..Default::default()
        },
    );
});
