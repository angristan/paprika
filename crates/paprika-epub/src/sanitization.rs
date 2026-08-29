use language_tags::LanguageTag;
use pulldown_cmark::{CowStr, Event, Options as MarkdownOptions, Parser, Tag, TagEnd, html};

use super::DEFAULT_LANGUAGE;

pub(super) fn replace_caption_paragraph_with_figure(
    body: &mut String,
    caption_marker: &str,
    image_source: &str,
    image_alt: &str,
) -> bool {
    let mut search_from = 0usize;
    while let Some(relative_start) = body[search_from..].find("<p") {
        let paragraph_start = search_from + relative_start;
        let Some(relative_open_end) = body[paragraph_start..].find('>') else {
            return false;
        };
        let content_start = paragraph_start + relative_open_end + 1;
        let Some(relative_close) = body[content_start..].find("</p>") else {
            return false;
        };
        let content_end = content_start + relative_close;
        let paragraph_end = content_end + "</p>".len();
        let caption_markup = &body[content_start..content_end];
        let caption_text = strip_markup(caption_markup);
        if caption_text.trim_start().starts_with(caption_marker) {
            let figure = format!(
                "<figure class=\"figure-fallback\"><img src=\"{image_source}\" alt=\"{image_alt}\"/><figcaption>{caption_markup}</figcaption></figure>"
            );
            body.replace_range(paragraph_start..paragraph_end, &figure);
            return true;
        }
        search_from = paragraph_end;
    }
    false
}

pub(super) fn replace_equation_anchor_with_image(
    body: &mut String,
    anchor: &str,
    image_source: &str,
    image_alt: &str,
) -> bool {
    let Some((paragraph_start, paragraph_end, content_start, anchor_start)) =
        find_equation_anchor_paragraph(body, anchor)
    else {
        return false;
    };
    let content_end = paragraph_end - "</p>".len();
    let content = &body[content_start..content_end];
    let prefix = content[..anchor_start].trim_end();
    let figure = format!(
        "<figure class=\"equation-fallback\"><img src=\"{image_source}\" alt=\"{image_alt}\"/></figure>"
    );
    let replacement = if strip_markup(prefix).trim().is_empty() {
        figure
    } else {
        format!(
            "{}{}</p>\n{figure}",
            &body[paragraph_start..content_start],
            prefix
        )
    };
    body.replace_range(paragraph_start..paragraph_end, &replacement);
    true
}

pub(super) fn find_equation_anchor_paragraph(
    body: &str,
    anchor: &str,
) -> Option<(usize, usize, usize, usize)> {
    let mut search_from = 0usize;
    let mut match_result = None;
    while let Some(relative_start) = body[search_from..].find("<p") {
        let paragraph_start = search_from + relative_start;
        let relative_open_end = body[paragraph_start..].find('>')?;
        let content_start = paragraph_start + relative_open_end + 1;
        let relative_close = body[content_start..].find("</p>")?;
        let content_end = content_start + relative_close;
        let paragraph_end = content_end + "</p>".len();
        let content = &body[content_start..content_end];
        let text = strip_markup(content);
        if text.trim_end().ends_with(anchor) {
            let anchor_start = content.rfind(anchor)?;
            if !content[anchor_start + anchor.len()..].trim().is_empty() {
                return None;
            }
            if match_result.is_some() {
                return None;
            }
            match_result = Some((paragraph_start, paragraph_end, content_start, anchor_start));
        }
        search_from = paragraph_end;
    }
    match_result
}

pub(super) fn enhance_algorithm_blocks(html: &mut String) {
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

pub(super) fn markdown_to_xhtml(markdown: &str) -> String {
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

pub(super) fn xhtml_document(title: &str, language: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{}\" lang=\"{}\">\n<head>\n<meta charset=\"UTF-8\"/>\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../styles.css\"/>\n</head>\n<body>\n{}</body>\n</html>\n",
        escape_xml(language),
        escape_xml(language),
        escape_xml(title),
        body
    )
}

pub(super) fn normalized_title(title: &str) -> String {
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

pub(super) fn normalized_language(language: &str) -> String {
    normalize_language_tag(language).unwrap_or_else(|| DEFAULT_LANGUAGE.to_string())
}

/// Parse, validate, and canonicalize a BCP 47 language tag.
pub(super) fn normalize_language_tag(language: &str) -> Option<String> {
    let language = language.trim();
    if language.is_empty() || language.len() > 255 {
        return None;
    }
    let parsed = LanguageTag::parse(language).ok()?;
    parsed.validate().ok()?;
    parsed.canonicalize().ok().map(LanguageTag::into_string)
}

pub(super) fn document_identifier(input: &[u8]) -> String {
    // Stable FNV-1a identifiers avoid random-number and clock dependencies in
    // both native and wasm32 builds. EPUB requires uniqueness, not cryptography.
    let hash = input.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("urn:paprika:{hash:016x}")
}

pub(super) fn first_heading(html: &str) -> Option<String> {
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

pub(super) fn no_text_warning(image_count: usize) -> &'static str {
    if image_count == 0 {
        "No selectable text or recoverable images were found; use OCR on the source PDF."
    } else {
        "No selectable text was found. This EPUB contains recoverable page imagery; use OCR for semantic output."
    }
}

pub(super) fn visible_text_len(html: &str) -> usize {
    strip_markup(html)
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
}

pub(super) fn strip_markup(html: &str) -> String {
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

pub(super) fn strip_invalid_xml_characters(value: &mut String) {
    value.retain(is_valid_xml_character);
}

fn is_valid_xml_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

pub(super) fn escape_xml(value: &str) -> String {
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
