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
    promote_run_in_section_headings(&mut output);
    output
}

fn promote_run_in_section_headings(html: &mut String) {
    const PARAGRAPH_START: &str = "<p>";
    const PARAGRAPH_END: &str = "</p>";
    let source = std::mem::take(html);
    let mut output = String::with_capacity(source.len());
    let mut search_from = 0usize;
    let mut container_depth = 0usize;

    while let Some(relative_start) = source[search_from..].find(PARAGRAPH_START) {
        let paragraph_start = search_from + relative_start;
        let content_start = paragraph_start + PARAGRAPH_START.len();
        let Some(relative_end) = source[content_start..].find(PARAGRAPH_END) else {
            break;
        };
        let content_end = content_start + relative_end;
        let paragraph_end = content_end + PARAGRAPH_END.len();
        let content = &source[content_start..content_end];

        let prefix = &source[search_from..paragraph_start];
        update_container_depth(prefix, &mut container_depth);
        output.push_str(prefix);
        let mut promoted = false;
        if container_depth == 0
            && let Some(strong_prefix) = leading_strong_prefix(content)
        {
            let body = content[strong_prefix.end..].trim_start();
            if is_section_heading(&strong_prefix, body) {
                output.push_str("<h2>");
                output.push_str(&escape_xml(promoted_heading_text(&strong_prefix.text)));
                output.push_str("</h2>");
                if !body.is_empty() {
                    output.push_str("\n<p>");
                    output.push_str(body);
                    output.push_str(PARAGRAPH_END);
                }
                promoted = true;
            }
        }
        if !promoted {
            output.push_str(&source[paragraph_start..paragraph_end]);
        }
        search_from = paragraph_end;
    }

    output.push_str(&source[search_from..]);
    *html = output;
}

pub(super) fn demote_heading(html: &mut String, level: u8, heading: &str) -> bool {
    if !(1..=6).contains(&level) {
        return false;
    }
    let heading = escape_xml(heading);
    let source = format!("<h{level}>{heading}</h{level}>");
    let matches: Vec<usize> = html.match_indices(&source).map(|(at, _)| at).collect();
    if matches.len() != 1 {
        return false;
    }
    let replacement = format!("<p><strong>{heading}</strong></p>");
    html.replace_range(matches[0]..matches[0] + source.len(), &replacement);
    true
}

pub(super) fn apply_reconstructed_heading(
    html: &mut String,
    level: u8,
    original: &str,
    reconstructed: &str,
    inserted_words: &[String],
    detached_context: &str,
) -> bool {
    if !(1..=6).contains(&level) || inserted_words.is_empty() || inserted_words.len() > 4 {
        return false;
    }
    let original_markup = format!("<h{level}>{}</h{level}>", escape_xml(original));
    let reconstructed_markup = format!("<h{level}>{}</h{level}>", escape_xml(reconstructed));
    let detached_markup = inserted_words
        .iter()
        .map(|word| format!("<strong>{}</strong>", escape_xml(word)))
        .collect::<Vec<_>>()
        .join(" ");
    let escaped_context = escape_xml(detached_context);
    let original_matches: Vec<usize> = html
        .match_indices(&original_markup)
        .map(|(at, _)| at)
        .collect();
    let detached_matches: Vec<usize> = html
        .match_indices(&escaped_context)
        .filter_map(|(context_start, _)| {
            let remainder_start = context_start + escaped_context.len();
            let remainder = &html[remainder_start..];
            let whitespace_bytes = remainder
                .char_indices()
                .take_while(|(_, character)| character.is_whitespace())
                .take(8)
                .last()
                .map_or(0, |(offset, character)| offset + character.len_utf8());
            remainder[whitespace_bytes..]
                .starts_with(&detached_markup)
                .then_some(remainder_start + whitespace_bytes)
        })
        .collect();
    if original_matches.len() != 1 || detached_matches.len() != 1 {
        return false;
    }
    let detached_start = detached_matches[0];

    let mut replacements = vec![
        (
            original_matches[0],
            original_matches[0] + original_markup.len(),
            reconstructed_markup,
        ),
        (
            detached_start,
            detached_start + detached_markup.len(),
            String::new(),
        ),
    ];
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0));
    for (start, end, replacement) in replacements {
        html.replace_range(start..end, &replacement);
    }
    true
}

pub(super) fn promote_positioned_run_in_headings(html: &mut String, headings: &[String]) {
    const PARAGRAPH_START: &str = "<p>";
    const PARAGRAPH_END: &str = "</p>";
    if headings.is_empty() {
        return;
    }

    let source = std::mem::take(html);
    let encoded_headings: Vec<(&str, String)> = headings
        .iter()
        .map(|heading| (heading.as_str(), escape_xml(heading)))
        .collect();
    let plain_heading_counts = positioned_plain_heading_counts(&source, &encoded_headings);
    let strong_heading_counts = positioned_strong_heading_counts(&source, &encoded_headings);
    let heading_occurrence_counts: Vec<usize> = plain_heading_counts
        .iter()
        .zip(&strong_heading_counts)
        .map(|(plain, strong)| plain.saturating_add(*strong))
        .collect();
    let unique_plain_headings: Vec<(&str, String)> = encoded_headings
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            plain_heading_counts[*index] == 1 && heading_occurrence_counts[*index] == 1
        })
        .map(|(_, heading)| heading.clone())
        .collect();

    let mut output = String::with_capacity(source.len() + encoded_headings.len() * 16);
    let mut search_from = 0usize;
    let mut container_depth = 0usize;
    while let Some(relative_start) = source[search_from..].find(PARAGRAPH_START) {
        let paragraph_start = search_from + relative_start;
        let content_start = paragraph_start + PARAGRAPH_START.len();
        let Some(relative_end) = source[content_start..].find(PARAGRAPH_END) else {
            break;
        };
        let content_end = content_start + relative_end;
        let paragraph_end = content_end + PARAGRAPH_END.len();
        let content = &source[content_start..content_end];
        let prefix = &source[search_from..paragraph_start];
        update_container_depth(prefix, &mut container_depth);
        output.push_str(prefix);

        let positioned_strong = (container_depth == 0).then(|| {
            positioned_strong_heading_match(content, &encoded_headings, &heading_occurrence_counts)
        });
        if let Some(Some(positioned)) = positioned_strong {
            let prefix_end = positioned.start + positioned.prefix.end;
            let body = content[prefix_end..].trim_start();
            push_paragraph_fragment(&mut output, &content[..positioned.start]);
            output.push_str("<h2>");
            output.push_str(&escape_xml(
                positioned
                    .heading
                    .trim_end_matches(['.', ':', ';', ','])
                    .trim_end(),
            ));
            output.push_str("</h2>");
            if !body.is_empty() {
                output.push_str("\n<p>");
                output.push_str(body);
                output.push_str(PARAGRAPH_END);
            }
        } else {
            let mut matches = if container_depth == 0 && !content.contains(['<', '>']) {
                positioned_heading_matches(content, &unique_plain_headings)
            } else {
                Vec::new()
            };
            if matches.is_empty() {
                output.push_str(&source[paragraph_start..paragraph_end]);
            } else {
                matches.sort_by_key(|candidate| candidate.start);
                let mut cursor = 0usize;
                for candidate in matches {
                    if candidate.start < cursor {
                        continue;
                    }
                    push_paragraph_fragment(&mut output, &content[cursor..candidate.start]);
                    output.push_str("<h2>");
                    output.push_str(&escape_xml(
                        candidate
                            .heading
                            .trim_end_matches(['.', ':', ';', ','])
                            .trim_end(),
                    ));
                    output.push_str("</h2>\n");
                    cursor = candidate.end;
                }
                push_paragraph_fragment(&mut output, &content[cursor..]);
                if output.ends_with('\n') {
                    output.pop();
                }
            }
        }
        search_from = paragraph_end;
    }

    output.push_str(&source[search_from..]);
    *html = output;
}

struct PositionedStrongHeadingMatch<'a> {
    start: usize,
    prefix: StrongPrefix,
    heading: &'a str,
}

fn positioned_plain_heading_counts(source: &str, headings: &[(&str, String)]) -> Vec<usize> {
    const PARAGRAPH_START: &str = "<p>";
    const PARAGRAPH_END: &str = "</p>";
    let mut counts = vec![0usize; headings.len()];
    let mut search_from = 0usize;
    let mut container_depth = 0usize;
    while let Some(relative_start) = source[search_from..].find(PARAGRAPH_START) {
        let paragraph_start = search_from + relative_start;
        let content_start = paragraph_start + PARAGRAPH_START.len();
        let Some(relative_end) = source[content_start..].find(PARAGRAPH_END) else {
            break;
        };
        let content_end = content_start + relative_end;
        let paragraph_end = content_end + PARAGRAPH_END.len();
        update_container_depth(&source[search_from..paragraph_start], &mut container_depth);
        let content = &source[content_start..content_end];
        if container_depth == 0 && !content.contains(['<', '>']) {
            for (index, (_, escaped)) in headings.iter().enumerate() {
                counts[index] = counts[index].saturating_add(content.matches(escaped).count());
            }
        }
        search_from = paragraph_end;
    }
    counts
}

fn positioned_strong_heading_counts(source: &str, headings: &[(&str, String)]) -> Vec<usize> {
    const PARAGRAPH_START: &str = "<p>";
    const PARAGRAPH_END: &str = "</p>";
    let mut counts = vec![0usize; headings.len()];
    let mut search_from = 0usize;
    let mut container_depth = 0usize;
    while let Some(relative_start) = source[search_from..].find(PARAGRAPH_START) {
        let paragraph_start = search_from + relative_start;
        let content_start = paragraph_start + PARAGRAPH_START.len();
        let Some(relative_end) = source[content_start..].find(PARAGRAPH_END) else {
            break;
        };
        let content_end = content_start + relative_end;
        let paragraph_end = content_end + PARAGRAPH_END.len();
        update_container_depth(&source[search_from..paragraph_start], &mut container_depth);
        if container_depth == 0 {
            visit_strong_prefixes(&source[content_start..content_end], |_, prefix| {
                for (index, (heading, _)) in headings.iter().enumerate() {
                    if *heading == prefix.text {
                        counts[index] = counts[index].saturating_add(1);
                    }
                }
            });
        }
        search_from = paragraph_end;
    }
    counts
}

fn positioned_strong_heading_match<'a>(
    content: &str,
    headings: &'a [(&'a str, String)],
    counts: &[usize],
) -> Option<PositionedStrongHeadingMatch<'a>> {
    let mut matched = None;
    let mut ambiguous = false;
    visit_strong_prefixes(content, |start, prefix| {
        let Some((heading, _)) = headings
            .iter()
            .enumerate()
            .find(|(index, (heading, _))| counts[*index] == 1 && *heading == prefix.text)
            .map(|(_, heading)| heading)
        else {
            return;
        };
        if matched.is_some() {
            ambiguous = true;
        } else {
            matched = Some(PositionedStrongHeadingMatch {
                start,
                prefix,
                heading,
            });
        }
    });
    (!ambiguous).then_some(matched).flatten()
}

fn visit_strong_prefixes(content: &str, mut visitor: impl FnMut(usize, StrongPrefix)) {
    const STRONG_START: &str = "<strong>";
    let mut cursor = 0usize;
    let mut depth_cursor = 0usize;
    let mut inline_depth = 0usize;
    while let Some(relative_start) = content[cursor..].find(STRONG_START) {
        let start = cursor + relative_start;
        update_inline_depth(&content[depth_cursor..start], &mut inline_depth);
        if inline_depth == 0
            && let Some(prefix) = leading_strong_prefix(&content[start..])
        {
            let next = start + prefix.end.max(STRONG_START.len());
            visitor(start, prefix);
            cursor = next;
        } else {
            cursor = start + STRONG_START.len();
        }
        depth_cursor = start;
    }
}

fn update_inline_depth(fragment: &str, depth: &mut usize) {
    const VOID_ELEMENTS: &[&str] = &["br", "hr", "img", "input"];
    let mut cursor = 0usize;
    while let Some(relative_start) = fragment[cursor..].find('<') {
        let tag_start = cursor + relative_start;
        let Some(relative_end) = fragment[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + relative_end;
        let tag = fragment[tag_start + 1..tag_end].trim();
        let closing = tag.starts_with('/');
        let name_start = usize::from(closing);
        let name = tag[name_start..]
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();
        if !name.is_empty() && !tag.starts_with('!') && !tag.starts_with('?') {
            if closing {
                *depth = depth.saturating_sub(1);
            } else if !tag.ends_with('/') && !VOID_ELEMENTS.contains(&name) {
                *depth = depth.saturating_add(1);
            }
        }
        cursor = tag_end + 1;
    }
}

#[derive(Clone, Copy, Debug)]
struct PositionedHeadingMatch<'a> {
    start: usize,
    end: usize,
    heading: &'a str,
}

fn positioned_heading_matches<'a>(
    content: &str,
    headings: &'a [(&'a str, String)],
) -> Vec<PositionedHeadingMatch<'a>> {
    let mut matches = Vec::new();
    for (heading, escaped) in headings {
        let Some(start) = content.find(escaped) else {
            continue;
        };
        let end = start + escaped.len();
        let starts_at_boundary = start == 0
            || content[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let ends_at_boundary = end == content.len()
            || content[end..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace);
        if starts_at_boundary && ends_at_boundary {
            matches.push(PositionedHeadingMatch {
                start,
                end,
                heading,
            });
        }
    }
    matches
}

fn push_paragraph_fragment(output: &mut String, fragment: &str) {
    let fragment = fragment.trim();
    if !fragment.is_empty() {
        output.push_str("<p>");
        output.push_str(fragment);
        output.push_str("</p>\n");
    }
}

#[derive(Debug)]
struct StrongPrefix {
    end: usize,
    text: String,
    parts: Vec<String>,
}

fn leading_strong_prefix(content: &str) -> Option<StrongPrefix> {
    const STRONG_START: &str = "<strong>";
    const STRONG_END: &str = "</strong>";
    const MAX_HEADING_PARTS: usize = 10;
    const MAX_PART_MARKUP_BYTES: usize = 512;
    let mut cursor = 0usize;
    let mut parts = Vec::new();

    loop {
        if !content[cursor..].starts_with(STRONG_START) {
            break;
        }
        if parts.len() == MAX_HEADING_PARTS {
            return None;
        }
        let value_start = cursor + STRONG_START.len();
        let value_end = value_start + content[value_start..].find(STRONG_END)?;
        if value_end - value_start > MAX_PART_MARKUP_BYTES {
            return None;
        }
        let text = strip_markup(&content[value_start..value_end]);
        if text.is_empty() {
            return None;
        }
        parts.push(text);
        cursor = value_end + STRONG_END.len();
        cursor += content[cursor..].len() - content[cursor..].trim_start().len();
    }

    (!parts.is_empty()).then(|| StrongPrefix {
        end: cursor,
        text: parts.join(" "),
        parts,
    })
}

fn is_section_heading(prefix: &StrongPrefix, body: &str) -> bool {
    const NON_SECTION_LABELS: &[&str] = &[
        "ALGORITHM",
        "CAUTION",
        "DEFINITION",
        "EQUATION",
        "EXAMPLE",
        "FIGURE",
        "IMPORTANT",
        "LEMMA",
        "NOTE",
        "PROOF",
        "REMARK",
        "TABLE",
        "THEOREM",
        "WARNING",
    ];
    let text = prefix.text.trim();
    let words: Vec<&str> = text.split_whitespace().collect();
    let letter_count = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    if text.chars().count() > 120 || !(4..=96).contains(&letter_count) || words.len() > 10 {
        return false;
    }

    let first_label = words
        .iter()
        .find(|word| word.chars().any(char::is_alphabetic))
        .map(|word| {
            word.trim_matches(|character: char| !character.is_alphabetic())
                .to_uppercase()
        });
    if first_label
        .as_deref()
        .is_some_and(|label| NON_SECTION_LABELS.contains(&label))
    {
        return false;
    }

    let starts_with_number = words.first().is_some_and(|word| {
        word.chars().any(|character| character.is_ascii_digit())
            && word
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, '.' | '-' | ':'))
    });
    let is_uppercase = text.chars().any(char::is_uppercase)
        && text
            .chars()
            .filter(|character| character.is_alphabetic())
            .all(|character| !character.is_lowercase());
    let is_plausible_unnumbered_heading = is_uppercase && (words.len() > 1 || letter_count >= 7);
    if starts_with_number || is_plausible_unnumbered_heading {
        return true;
    }

    if is_appendix_heading(prefix, body) {
        return true;
    }

    let normalized = normalized_heading_label(text);
    let is_known_label = matches!(
        normalized.as_str(),
        "ABSTRACT" | "ACKNOWLEDGEMENTS" | "ACKNOWLEDGMENTS" | "DECODER" | "ENCODER"
    );
    let body_summary = visible_text_summary(body, 256);
    is_known_label && body_summary.letters >= 24 && body_summary.words >= 5
}

fn is_appendix_heading(prefix: &StrongPrefix, body: &str) -> bool {
    if !body.trim().is_empty() || prefix.parts.len() < 3 {
        return false;
    }
    let mut words = prefix.text.split_whitespace();
    let Some(designator) = words.next() else {
        return false;
    };
    let remaining: Vec<&str> = words.collect();
    remaining.len() >= 2
        && is_appendix_designator(designator)
        && remaining
            .iter()
            .find_map(|word| word.chars().find(|character| character.is_alphabetic()))
            .is_some_and(char::is_uppercase)
}

fn is_appendix_designator(value: &str) -> bool {
    let mut components = value.split('.');
    let Some(section) = components.next() else {
        return false;
    };
    if section.len() != 1 || !section.as_bytes()[0].is_ascii_uppercase() {
        return false;
    }
    components.all(|component| {
        !component.is_empty()
            && component
                .chars()
                .all(|character| character.is_ascii_digit())
    })
}

fn promoted_heading_text(text: &str) -> &str {
    if matches!(
        normalized_heading_label(text).as_str(),
        "ABSTRACT" | "ACKNOWLEDGEMENTS" | "ACKNOWLEDGMENTS" | "DECODER" | "ENCODER"
    ) {
        text.trim_end_matches(['.', ':', ';', ',']).trim_end()
    } else {
        text
    }
}

fn normalized_heading_label(text: &str) -> String {
    text.trim()
        .trim_end_matches(['.', ':', ';', ','])
        .to_uppercase()
}

#[derive(Clone, Copy, Debug, Default)]
struct VisibleTextSummary {
    letters: usize,
    words: usize,
}

fn visible_text_summary(markup: &str, maximum_visible_characters: usize) -> VisibleTextSummary {
    let mut summary = VisibleTextSummary::default();
    let mut inside_tag = false;
    let mut inside_entity = false;
    let mut inside_word = false;
    let mut visible_characters = 0usize;

    for character in markup.chars() {
        match character {
            '<' if !inside_entity => {
                inside_tag = true;
                inside_word = false;
                continue;
            }
            '>' if inside_tag => {
                inside_tag = false;
                continue;
            }
            '&' if !inside_tag => {
                inside_entity = true;
                inside_word = false;
                continue;
            }
            ';' if inside_entity => {
                inside_entity = false;
                visible_characters += 1;
            }
            _ if inside_tag || inside_entity => continue,
            _ => visible_characters += 1,
        }
        if visible_characters > maximum_visible_characters {
            break;
        }
        if character.is_alphabetic() {
            summary.letters += 1;
            if !inside_word {
                summary.words += 1;
                inside_word = true;
            }
        } else {
            inside_word = false;
        }
    }
    summary
}

fn update_container_depth(fragment: &str, depth: &mut usize) {
    const CONTAINERS: &[&str] = &["blockquote", "div", "figure", "li", "td", "th"];
    let mut cursor = 0usize;
    while let Some(relative_start) = fragment[cursor..].find('<') {
        let tag_start = cursor + relative_start;
        let Some(relative_end) = fragment[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + relative_end;
        let tag = fragment[tag_start + 1..tag_end].trim();
        let closing = tag.starts_with('/');
        let name_start = usize::from(closing);
        let name = tag[name_start..]
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();
        if CONTAINERS.contains(&name) {
            if closing {
                *depth = depth.saturating_sub(1);
            } else if !tag.ends_with('/') {
                *depth = depth.saturating_add(1);
            }
        }
        cursor = tag_end + 1;
    }
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

pub(super) fn headings_in_document_order(html: &str) -> Vec<(u8, String)> {
    let mut headings = Vec::new();
    for level in 1..=6u8 {
        let open = format!("<h{level}>");
        let close = format!("</h{level}>");
        let mut search_from = 0usize;
        while let Some(relative_start) = html[search_from..].find(&open) {
            let start = search_from + relative_start;
            let content_start = start + open.len();
            let Some(relative_end) = html[content_start..].find(&close) else {
                break;
            };
            let content_end = content_start + relative_end;
            let text = strip_markup(&html[content_start..content_end]);
            if text
                .chars()
                .filter(|character| character.is_alphanumeric())
                .count()
                >= 2
            {
                headings.push((start, level, text.chars().take(120).collect()));
            }
            search_from = content_end + close.len();
        }
    }
    headings.sort_unstable_by_key(|(position, _, _)| *position);
    headings
        .into_iter()
        .map(|(_, level, text)| (level, text))
        .collect()
}

pub(super) fn first_heading(html: &str) -> Option<String> {
    headings_in_document_order(html)
        .into_iter()
        .next()
        .map(|(_, heading)| heading)
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
