use rbook::Epub;
use rbook::ebook::toc::TocEntryKind;
use rbook::epub::EpubChapter;
use rbook::epub::metadata::EpubVersion;
use rbook::epub::toc::DetachedEpubTocEntry;

use super::buffer::BoundedBuffer;
use super::extraction::render_semantic_page_body;
use super::model::SemanticPage;
use super::sanitization::xhtml_document;
use super::{
    EPUB_CSS, EpubError, EpubPreview, EpubPreviewAsset, EpubPreviewChapter, EpubPreviewLimits,
};

pub(super) fn build_epub_preview(
    pages: &[SemanticPage],
    language: &str,
    limits: EpubPreviewLimits,
) -> Result<EpubPreview, EpubError> {
    let mut chapters = Vec::new();
    let mut assets = Vec::new();
    let mut xhtml_bytes = 0usize;
    let mut asset_bytes = 0usize;

    for page in pages.iter().take(limits.max_chapters) {
        let estimated_markup_bytes = page
            .images
            .iter()
            .map(|image| {
                image
                    .href
                    .len()
                    .saturating_add(image.alt.len())
                    .saturating_mul(6)
                    .saturating_add(512)
            })
            .sum::<usize>()
            .saturating_add(page.html.len())
            .saturating_add(page.title.len().saturating_mul(6))
            .saturating_add(1024);
        if xhtml_bytes.saturating_add(estimated_markup_bytes) > limits.max_xhtml_bytes {
            break;
        }
        let page_asset_bytes = page
            .images
            .iter()
            .map(|image| image.bytes.len())
            .sum::<usize>();
        let next_asset_bytes = asset_bytes.saturating_add(page_asset_bytes);
        if next_asset_bytes > limits.max_asset_bytes
            || assets.len().saturating_add(page.images.len()) > limits.max_assets
        {
            break;
        }

        let body = render_semantic_page_body(page)?;
        let xhtml = xhtml_document(&page.title, language, &body);
        let next_xhtml_bytes = xhtml_bytes.saturating_add(xhtml.len());
        if next_xhtml_bytes > limits.max_xhtml_bytes {
            break;
        }
        xhtml_bytes = next_xhtml_bytes;
        asset_bytes = next_asset_bytes;
        chapters.push(EpubPreviewChapter {
            source_page: page.number,
            title: page.title.clone(),
            href: format!("text/page-{:04}.xhtml", page.number),
            xhtml,
        });
        assets.extend(page.images.iter().map(|image| EpubPreviewAsset {
            href: image.href.clone(),
            media_type: image_media_type(&image.href).to_string(),
            bytes: image.bytes.clone(),
        }));
    }

    Ok(EpubPreview {
        stylesheet: EPUB_CSS.to_string(),
        truncated: chapters.len() < pages.len(),
        chapters,
        assets,
    })
}

fn image_media_type(href: &str) -> &'static str {
    if href.ends_with(".jpg") || href.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "image/png"
    }
}

pub(super) fn package_epub(
    title: &str,
    language: &str,
    identifier: &str,
    pages: Vec<SemanticPage>,
    max_semantic_bytes: usize,
    max_output_bytes: usize,
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
    let mut semantic_bytes = 0usize;
    let mut page_list = Vec::with_capacity(pages.len());

    for page in pages {
        let chapter_href = format!("text/page-{:04}.xhtml", page.number);
        let body = render_semantic_page_body(&page)?;
        for image in page.images {
            editor = editor.resource((image.href, image.bytes));
        }

        let xhtml = xhtml_document(&page.title, language, &body);
        semantic_bytes =
            semantic_bytes
                .checked_add(xhtml.len())
                .ok_or(EpubError::SemanticTooLarge {
                    limit: max_semantic_bytes / (1024 * 1024),
                })?;
        if semantic_bytes > max_semantic_bytes {
            return Err(EpubError::SemanticTooLarge {
                limit: max_semantic_bytes / (1024 * 1024),
            });
        }
        page_list.push(
            DetachedEpubTocEntry::new(page.number.to_string())
                .href(format!("{chapter_href}#page-{}", page.number)),
        );
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

    let mut epub = editor.build();
    epub.toc_mut().insert_root(
        TocEntryKind::PageList,
        EpubVersion::EPUB3,
        DetachedEpubTocEntry::new("Pages").children(page_list),
    );

    let mut output = BoundedBuffer::new(max_output_bytes);
    let result = epub
        .write()
        // Moderate compression keeps image-heavy EPUBs compact without the
        // disproportionate CPU cost of the previous maximum setting.
        .compression(6)
        .toc_stylesheet("styles.css")
        .write(&mut output);
    let package_error = result.err().map(|error| error.to_string());
    if output.limit_exceeded {
        return Err(EpubError::OutputTooLarge {
            limit: max_output_bytes / (1024 * 1024),
        });
    }
    if let Some(error) = package_error {
        return Err(EpubError::Package(error));
    }
    Ok(output.into_inner())
}
