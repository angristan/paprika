use super::*;
use crate::buffer::BoundedBuffer;
use crate::equations::find_display_equations;
use crate::extraction::{
    RepeatedRunningText, account_rendered_xhtml, equation_anchors_are_unique,
    strip_repeated_running_html,
};
use crate::figures::{
    figure_graphic_region, page_may_have_figure_caption, tighten_regions_from_graphics,
};
use crate::geometry::{
    MAX_TITLE_SPANS, overlap_fraction, reconstruct_document_title, visual_page_regions,
};
use crate::images::{
    ImageDecodeBudget, account_image_objects, image_recovery_plan, reserve_image_decode,
};
use crate::math::{is_math_dense_candidate, math_extraction_is_unreliable, trustworthy_prose_html};
use crate::model::{ImagePlacement, PageImage, SemanticPage};
use crate::packaging::{build_epub_preview, package_epub};
use crate::sanitization::{
    enhance_algorithm_blocks, escape_xml, markdown_to_xhtml, no_text_warning, normalized_language,
    normalized_title, replace_caption_paragraph_with_figure, replace_equation_anchor_with_image,
    xhtml_document,
};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use rbook::Epub;
use std::collections::HashSet;
use std::io::Write;

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
fn separates_run_in_section_headings_from_body_text() {
    let html = markdown_to_xhtml(
        "**9** **CONCLUSIONS** When queues are needed, local storage helps.\n\n**ACKNOWLEDGMENTS** We thank the contributors.",
    );
    assert!(
        html.contains(
            "<h2>9 CONCLUSIONS</h2>\n<p>When queues are needed, local storage helps.</p>"
        )
    );
    assert!(html.contains("<h2>ACKNOWLEDGMENTS</h2>\n<p>We thank the contributors.</p>"));
}

#[test]
fn preserves_bold_lead_ins_and_figure_captions_as_paragraphs() {
    let html = markdown_to_xhtml(
        "**Important** context remains inline.\n\n**NOTE** Keep this callout inline.\n\n**HTTP** requests remain prose.\n\n**FIGURE** **1:** Sample graph.",
    );
    assert!(html.contains("<p><strong>Important</strong> context remains inline.</p>"));
    assert!(html.contains("<p><strong>NOTE</strong> Keep this callout inline.</p>"));
    assert!(html.contains("<p><strong>HTTP</strong> requests remain prose.</p>"));
    assert!(html.contains("<p><strong>FIGURE</strong> <strong>1:</strong> Sample graph.</p>"));
    assert!(!html.contains("<h2>"));
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
    output.write_all(b"four").unwrap();
    assert!(output.limit_exceeded);
    assert!(output.into_inner().is_empty());
}

#[test]
fn enforces_per_page_and_cumulative_image_limits() {
    let mut objects = ImageDecodeBudget::default();
    assert!(matches!(
        account_image_objects(&mut objects, 0, MAX_IMAGE_OBJECTS_PER_PAGE + 1),
        Err(EpubError::TooManyImageObjects { page: 1, .. })
    ));

    let mut cumulative = ImageDecodeBudget {
        objects: MAX_IMAGE_OBJECTS_TOTAL,
        pixels: 0,
    };
    assert!(matches!(
        account_image_objects(&mut cumulative, 1, 1),
        Err(EpubError::TooManyImageObjectsTotal { .. })
    ));

    let mut decode = ImageDecodeBudget::default();
    let mut page_pixels = MAX_IMAGE_DECODE_PIXELS_PER_PAGE;
    assert!(matches!(
        reserve_image_decode(&mut decode, 2, &mut page_pixels, 1),
        Err(EpubError::PageImageDecodeLimit { page: 3, .. })
    ));

    let mut cumulative_decode = ImageDecodeBudget {
        objects: 0,
        pixels: MAX_IMAGE_DECODE_PIXELS_TOTAL,
    };
    let mut page_pixels = 0;
    assert!(matches!(
        reserve_image_decode(&mut cumulative_decode, 0, &mut page_pixels, 1),
        Err(EpubError::ImageDecodeLimit { .. })
    ));
}

#[test]
fn downsamples_recoverable_full_page_scans() {
    let page = Rect::new(0.0, 0.0, 612.0, 792.0);
    let scan = Rect::new(0.0, 0.0, 612.0, 792.0);
    let plan = image_recovery_plan(3_000, 2_000, scan, true, Some(page)).unwrap();
    assert!(plan.downsampled);
    assert!(plan.visual_page);
    assert!(u64::from(plan.width) * u64::from(plan.height) <= MAX_IMAGE_PIXELS);
    assert!(image_recovery_plan(3_000, 2_000, scan, false, Some(page)).is_none());
    assert!(image_recovery_plan(8_000, 8_000, scan, true, Some(page)).is_none());
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
        (overlap_fraction(image, Rect::new(60.0, 10.0, 100.0, 100.0)) - 0.5).abs() < f32::EPSILON
    );
}

#[test]
fn ignores_figure_captions_without_graphic_geometry() {
    let coarse = Rect::new(59.0, 524.0, 227.0, 225.0);
    assert!(figure_graphic_region(coarse, &[], &[]).is_none());
    assert!(figure_graphic_region(coarse, &[Rect::new(80.0, 540.0, 120.0, 80.0)], &[]).is_some());
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
    sized_test_span(text, x, y, width, 10.0, font)
}

fn sized_test_span(text: &str, x: f32, y: f32, width: f32, font_size: f32, font: &str) -> TextSpan {
    TextSpan {
        text: text.to_string(),
        bbox: Rect::new(x, y, width, font_size),
        font_name: font.to_string(),
        font_size,
        ..Default::default()
    }
}

#[test]
fn reconstructs_a_title_split_at_the_column_boundary() {
    let spans = vec![
        sized_test_span("QuiCK: A Queuing", 154.0, 696.0, 147.0, 17.2, "Times"),
        sized_test_span("System in CloudKit", 306.0, 696.0, 151.0, 17.2, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "QuiCK: A Queuing",
        &[],
        "QuiCK: A Queuing System in CloudKit",
    )
    .unwrap();
    assert_eq!(title.text, "QuiCK: A Queuing System in CloudKit");
    assert!(title.bbox.x < 154.0);
    assert!(title.bbox.x + title.bbox.width > 457.0);
}

#[test]
fn reconstructs_a_wrapped_title_without_including_authors() {
    let spans = vec![
        sized_test_span(
            "BERT: Pre-training of Deep Bidirectional Transformers for",
            116.0,
            760.0,
            365.0,
            14.3,
            "Times",
        ),
        sized_test_span("Language Understanding", 221.0, 744.0, 156.0, 14.3, "Times"),
        sized_test_span("Jacob Devlin", 122.0, 702.0, 66.0, 12.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "BERT: Pre-training",
        &["Language Understanding".to_string()],
        "BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding",
    )
    .unwrap();
    assert_eq!(
        title.text,
        "BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding"
    );
    assert!(!title.text.contains("Jacob"));
}

#[test]
fn preserves_standalone_title_punctuation() {
    let spans = vec![
        sized_test_span("Research", 100.0, 700.0, 70.0, 16.0, "Times"),
        sized_test_span("&", 176.0, 700.0, 10.0, 16.0, "Times"),
        sized_test_span("Practice", 192.0, 700.0, 66.0, 16.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "Research &",
        &[],
        "Research & Practice",
    )
    .unwrap();
    assert_eq!(title.text, "Research & Practice");
}

#[test]
fn does_not_join_independent_column_headings() {
    let spans = vec![
        sized_test_span("Left heading", 50.0, 700.0, 100.0, 16.0, "Times"),
        sized_test_span("Right heading", 350.0, 700.0, 110.0, 16.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "Left heading",
        &[],
        "Left heading",
    )
    .unwrap();
    assert_eq!(title.text, "Left heading");
    assert!(title.bbox.x + title.bbox.width < 200.0);
}

#[test]
fn does_not_absorb_a_similar_sized_byline() {
    let spans = vec![
        sized_test_span("A Complete", 160.0, 700.0, 100.0, 16.0, "Times"),
        sized_test_span("Title", 265.0, 700.0, 40.0, 16.0, "Times"),
        // Some extractors classify a centered byline at the same heading level
        // and size. A split first line must still stop before this author row.
        sized_test_span("Ada Author", 195.0, 684.0, 110.0, 16.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "A Complete",
        &["Ada Author".to_string()],
        "A Complete Title",
    )
    .unwrap();
    assert_eq!(title.text, "A Complete Title");
}

#[test]
fn stops_after_metadata_confirmed_wrapped_title() {
    let spans = vec![
        sized_test_span("Study", 160.0, 700.0, 50.0, 16.0, "Times"),
        sized_test_span("for", 215.0, 700.0, 24.0, 16.0, "Times"),
        sized_test_span("Language Understanding", 170.0, 684.0, 170.0, 16.0, "Times"),
        sized_test_span("Ada Author", 195.0, 668.0, 110.0, 16.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "Study",
        &[
            "Language Understanding".to_string(),
            "Ada Author".to_string(),
        ],
        "Study for Language Understanding",
    )
    .unwrap();
    assert_eq!(title.text, "Study for Language Understanding");
}

#[test]
fn ignores_title_geometry_outside_page_bounds() {
    let spans = vec![
        sized_test_span("Safe Title", 160.0, 700.0, 120.0, 16.0, "Times"),
        sized_test_span("Oversized", 1.0, 700.0, f32::MAX, 16.0, "Times"),
    ];
    let title = reconstruct_document_title(
        &spans,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        "Safe Title",
        &[],
        "Safe Title",
    )
    .unwrap();
    assert_eq!(title.text, "Safe Title");
    assert!(title.bbox.x >= 0.0);
    assert!(title.bbox.x + title.bbox.width <= 612.0);
}

#[test]
fn refuses_excessive_title_geometry() {
    let spans = (0..=MAX_TITLE_SPANS)
        .map(|index| sized_test_span("Title", (index % 100) as f32, 700.0, 20.0, 16.0, "Times"))
        .collect::<Vec<_>>();
    assert!(
        reconstruct_document_title(
            &spans,
            Rect::new(0.0, 0.0, 612.0, 792.0),
            "Title",
            &[],
            "Title",
        )
        .is_none()
    );
}

#[test]
fn prefilters_varied_figure_caption_tokens() {
    assert!(page_may_have_figure_caption(&[test_span(
        "FIGURE 2: Sample",
        10.0,
        10.0,
        100.0,
        "Times",
    )]));
    assert!(page_may_have_figure_caption(&[test_span(
        "Figure: 2 Sample",
        10.0,
        10.0,
        100.0,
        "Times",
    )]));
    assert!(!page_may_have_figure_caption(&[test_span(
        "Configuration details",
        10.0,
        10.0,
        100.0,
        "Times",
    )]));
}

#[test]
fn retains_plain_prose_but_not_formula_markup_for_visual_fallback() {
    let html = "<p>This explanatory paragraph remains readable and useful.</p>\n<p>f(x) = y + z</p>\n<h2>Method overview</h2>";
    let preserved = trustworthy_prose_html(html);
    assert!(preserved.contains("explanatory paragraph"));
    assert!(preserved.contains("Method overview"));
    assert!(!preserved.contains("f(x)"));
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
        find_display_equations(&spans, Rect::new(0.0, 0.0, 612.0, 792.0), &[occupied]).is_empty()
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
fn strips_precomputed_running_headers_from_xhtml() {
    let repeated = RepeatedRunningText {
        headers: HashSet::from(["journal title".to_string()]),
        footers: HashSet::from(["copyright".to_string()]),
    };
    let mut html = "<p>Journal Title 12</p>\n<p>Journal Title 99</p>\n<p>Introduction.</p>\n<p>Keep this body paragraph.</p>\n<p>Copyright 2025</p>\n<p>Copyright 2026</p>".to_string();
    strip_repeated_running_html(&mut html, &repeated);
    assert_eq!(html.matches("Journal Title").count(), 1);
    assert_eq!(html.matches("Copyright").count(), 1);
    assert!(html.contains("Keep this body paragraph."));
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
    assert_eq!(normalized_language("zh-Hant-TW"), "zh-Hant-TW");
    assert_eq!(normalized_language("x-private"), "x-private");
    for malformed in ["en<script>", "en--US", "-en", "en-", "e", "en-toolongtag"] {
        assert_eq!(normalized_language(malformed), "en", "accepted {malformed}");
    }
}

#[test]
fn accounts_for_the_complete_rendered_xhtml() {
    let page = SemanticPage {
        number: 1,
        title: "Chapter".to_string(),
        html: "<p>x</p>".to_string(),
        images: Vec::new(),
        has_text: true,
    };
    let mut bytes = 0;
    assert!(matches!(
        account_rendered_xhtml(&page, "en", &mut bytes, page.html.len()),
        Err(EpubError::SemanticTooLarge { .. })
    ));
}

#[test]
fn distinguishes_empty_and_image_only_books() {
    assert!(no_text_warning(0).contains("or recoverable images"));
    assert!(!no_text_warning(1).contains("or recoverable images"));
}

#[test]
fn emits_valid_xhtml_shell_and_escapes_metadata() {
    let xhtml = xhtml_document("A & B", "en", "<p>Body</p>");
    assert!(xhtml.contains("<title>A &amp; B</title>"));
    assert!(xhtml.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
    assert!(xhtml.contains("<p>Body</p>"));
}

#[test]
fn builds_bounded_preview_from_packaged_page_model() {
    let page = SemanticPage {
        number: 3,
        title: "Formula page".to_string(),
        html: "<p>Readable context.</p>".to_string(),
        images: vec![PageImage {
            href: "images/page-0003-equation-01.png".to_string(),
            bytes: vec![1, 2, 3],
            alt: "Display equation".to_string(),
            placement: ImagePlacement::EndOfPage,
        }],
        has_text: true,
    };
    let preview = build_epub_preview(
        &[page],
        "en",
        EpubPreviewLimits {
            max_chapters: 1,
            max_xhtml_bytes: 4096,
            max_asset_bytes: 4096,
            max_assets: 1,
        },
    )
    .unwrap();
    assert_eq!(preview.chapters.len(), 1);
    assert_eq!(preview.assets.len(), 1);
    assert_eq!(preview.assets[0].bytes, [1, 2, 3]);
    assert!(
        preview.chapters[0]
            .xhtml
            .contains("../images/page-0003-equation-01.png")
    );
    assert!(!preview.truncated);
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
    let bytes = package_epub(
        "Test",
        "en",
        "urn:paprika:test",
        vec![page],
        usize::MAX,
        usize::MAX,
    )
    .unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert!(
        bytes
            .windows(b"application/epub+zip".len())
            .any(|window| window == b"application/epub+zip")
    );
    let epub = Epub::read(std::io::Cursor::new(bytes)).unwrap();
    let chapter = epub.read_resource_str("text/page-0001.xhtml").unwrap();
    assert!(chapter.contains("Select me"));
    assert!(chapter.contains("<title>Introduction</title>"));
    assert!(chapter.contains("epub:type=\"pagebreak\""));
    assert!(chapter.contains("id=\"page-1\""));
    assert_eq!(epub.toc().page_list().unwrap().len(), 1);
}

#[test]
fn bounds_final_epub_writes() {
    let page = SemanticPage {
        number: 1,
        title: "Introduction".to_string(),
        html: "<p>Select me</p>".to_string(),
        images: Vec::new(),
        has_text: true,
    };
    assert!(matches!(
        package_epub("Test", "en", "urn:paprika:test", vec![page], usize::MAX, 64,),
        Err(EpubError::OutputTooLarge { .. })
    ));
}
