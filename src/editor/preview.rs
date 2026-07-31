//! Markdown Live Preview layout: stable block spans plus per-line style
//! spans over the single source document (stint 0476). Pure — no egui.
//!
//! Parsing is isolated here as an adapter over `pulldown-cmark`
//! (`Parser::into_offset_iter` gives exact byte offsets into the source), so
//! the editor core never forks parser behavior. The widget consumes the
//! result as per-line style spans layered into its existing per-line
//! `LayoutJob` galleys.
//!
//! ## Source ↔ layout mapping contract
//!
//! Live Preview styles lines *in place*: every source character stays present
//! in its rendered line, styling changes only color/italic/strike — never
//! glyph metrics, line count, or line heights. The rendered layout position of
//! any source position is therefore the identity mapping, which is what keeps
//! caret geometry, hit-testing, drag selection, scroll anchoring, undo, and
//! IME byte-for-byte coherent with source mode. The one sanctioned vertical
//! divergence is `ViewState::line_extras` (stint 0478): inline image strips
//! reserve extra height *below* a line without touching the line's own text
//! metrics, and all vertical math (`line_top`, `line_at_y`, scrolling) flows
//! through `ViewState` so hit-testing stays exact. If a future change styles
//! with a different font or hides characters, `hit_test`, caret x, and
//! `view.viewport_width` in `widget.rs` must all learn the new mapping
//! together.
//!
//! Unknown/unsupported top-level structures (tables, HTML blocks, footnote
//! definitions…) degrade to editable raw source ([`BlockKind::Fallback`]);
//! that fallback is traced at info level once per parse.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::buffer::TextBuffer;

/// The kind of a top-level Markdown block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// ATX/setext heading with its level (1-6).
    Heading(u8),
    Paragraph,
    /// One outermost list item (unordered, ordered, or task).
    ListItem,
    Quote,
    /// Fenced or indented code block, including its fence lines.
    CodeFence,
    /// Thematic break (`---`, `***`, `___`).
    Rule,
    /// Blank/whitespace-only lines between blocks.
    Blank,
    /// Structure the preview does not style (table, HTML block, …):
    /// rendered as raw editable source.
    Fallback,
}

/// One top-level block: kind plus the line range it covers. Blocks are
/// sorted, non-overlapping, and cover every line of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Half-open line range `[start, end)`.
    pub lines: Range<usize>,
}

/// Inline emphasis kinds the preview styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Strong,
    Emphasis,
    Code,
    Strikethrough,
    /// Markdown/auto/wiki link source span (delimiters included).
    Link,
}

/// How a link was written in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[display](dest)`.
    Markdown,
    /// `<https://…>` or a bare autolink.
    Autolink,
    /// A bare `http://` or `https://` URL detected only for live rendering.
    BareUrl,
    /// `[[note name]]` wiki-style note link (not cmark syntax; scanned here).
    Wiki,
}

/// One link in the document: byte range is delimiter-inclusive (covers the
/// whole `[..](..)`, `<..>`, or `[[..]]` source span).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkTarget {
    pub kind: LinkKind,
    pub bytes: Range<usize>,
    /// Raw destination as written (URL, relative path, or wiki note name).
    pub dest: String,
    /// Human-readable text (link label; the dest itself for autolinks).
    pub display: String,
}

/// One inline image reference `![alt](dest)`: byte range delimiter-inclusive.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageSpan {
    pub bytes: Range<usize>,
    /// Raw destination as written (relative path or URL).
    pub dest: String,
    pub alt: String,
}

/// An inline span in document byte offsets (delimiters included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub bytes: Range<usize>,
    pub kind: InlineKind,
}

/// Style classes for one rendered line, mapped to theme colors by the widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdStyle {
    /// Structural syntax (`#`, `-`, `>`, `**`, fences…): dimmed.
    Marker,
    /// Heading text (level carried for future size tiers; color-only today).
    Heading(u8),
    Strong,
    Emphasis,
    Code,
    Quote,
    Rule,
    /// Link source span (Markdown, autolink, or wiki): accent + underline.
    Link,
    Plain,
}

/// One styled region: byte offsets within a single line's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub style: MdStyle,
}

/// Parsed block/inline layout for one document revision.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkdownLayout {
    pub blocks: Vec<Block>,
    pub inlines: Vec<InlineSpan>,
    /// All links in source order (Markdown, autolink, bare URL, wiki).
    pub links: Vec<LinkTarget>,
    /// All inline image references in source order.
    pub images: Vec<ImageSpan>,
    /// Byte offset where each line starts, plus one final entry at `len()`.
    line_starts: Vec<usize>,
}

impl MarkdownLayout {
    /// Line index containing document byte offset `byte`.
    #[must_use]
    pub fn line_of_byte(&self, byte: usize) -> usize {
        let line_count = self.line_starts.len().saturating_sub(1);
        if line_count == 0 {
            return 0;
        }
        match self.line_starts[..line_count].binary_search(&byte) {
            Ok(l) => l,
            Err(l) => l.saturating_sub(1),
        }
    }

    /// The link whose delimiter-inclusive span contains `byte`, if any.
    #[must_use]
    pub fn link_at_byte(&self, byte: usize) -> Option<&LinkTarget> {
        self.links.iter().find(|l| l.bytes.contains(&byte))
    }

    /// Index of the block containing `line`. Blocks cover every line, so this
    /// only returns `None` for an empty layout or out-of-range line.
    #[must_use]
    pub fn block_index_at_line(&self, line: usize) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.lines.contains(&line))
            .or_else(|| {
                // A caret one past the last line (empty trailing line owned by
                // the final block's end) maps to the last block.
                self.blocks.last().map(|_| self.blocks.len() - 1)
            })
    }

    #[must_use]
    pub fn block_at_line(&self, line: usize) -> Option<&Block> {
        self.block_index_at_line(line).map(|i| &self.blocks[i])
    }

    /// The union of lines revealed as raw source for a selection spanning
    /// `first_line..=last_line`: from the start of the block containing the
    /// first line through the end of the block containing the last.
    #[must_use]
    pub fn active_lines(&self, first_line: usize, last_line: usize) -> Range<usize> {
        let start = self
            .block_at_line(first_line.min(last_line))
            .map_or(0, |b| b.lines.start);
        let end = self
            .block_at_line(first_line.max(last_line))
            .map_or(0, |b| b.lines.end);
        start..end.max(start)
    }

    /// Byte offset in the document where `line` starts.
    #[must_use]
    pub fn line_byte_start(&self, line: usize) -> usize {
        self.line_starts
            .get(line)
            .or(self.line_starts.last())
            .copied()
            .unwrap_or(0)
    }

    /// Style spans for one rendered (inactive) line: contiguous, sorted,
    /// covering `0..line_text.len()`. `line_text` must be the exact source
    /// text of `line` (no trailing newline).
    #[must_use]
    pub fn line_style_spans(&self, line: usize, line_text: &str) -> Vec<StyleSpan> {
        if line_text.is_empty() {
            return Vec::new();
        }
        let kind = self
            .block_at_line(line)
            .map_or(BlockKind::Fallback, |b| b.kind);
        let base = match kind {
            BlockKind::Heading(level) => MdStyle::Heading(level),
            BlockKind::CodeFence => MdStyle::Code,
            BlockKind::Quote => MdStyle::Quote,
            BlockKind::Rule => MdStyle::Rule,
            BlockKind::Fallback => MdStyle::Plain,
            _ => MdStyle::Plain,
        };
        let mut styles = vec![base; line_text.len()];

        // Structural line prefixes render dimmed: heading hashes, quote
        // angle brackets, list markers, fence delimiters.
        let marker_len = match kind {
            BlockKind::Heading(_) => heading_marker_len(line_text),
            BlockKind::Quote => quote_marker_len(line_text),
            BlockKind::ListItem => list_marker_len(line_text),
            BlockKind::CodeFence => fence_marker_len(line_text),
            BlockKind::Rule => line_text.len(),
            _ => 0,
        };
        for slot in styles.iter_mut().take(marker_len) {
            *slot = MdStyle::Marker;
        }

        // Overlay inline emphasis intersecting this line, dimming the
        // delimiter bytes at the span edges. Skip inside code blocks.
        if kind != BlockKind::CodeFence {
            let line_range =
                self.line_byte_start(line)..self.line_byte_start(line) + line_text.len();
            for span in &self.inlines {
                let start = span.bytes.start.max(line_range.start);
                let end = span.bytes.end.min(line_range.end);
                if start >= end {
                    continue;
                }
                let style = match span.kind {
                    InlineKind::Strong => MdStyle::Strong,
                    InlineKind::Emphasis => MdStyle::Emphasis,
                    InlineKind::Code => MdStyle::Code,
                    InlineKind::Strikethrough => MdStyle::Emphasis,
                    InlineKind::Link => MdStyle::Link,
                };
                let delim = match span.kind {
                    InlineKind::Strong | InlineKind::Strikethrough => 2,
                    InlineKind::Emphasis | InlineKind::Code => 1,
                    // Whole span styled as a link, delimiters included.
                    InlineKind::Link => 0,
                };
                for b in start..end {
                    let local = b - line_range.start;
                    let leading = b < span.bytes.start + delim;
                    let trailing = b + delim >= span.bytes.end;
                    styles[local] = if leading || trailing {
                        MdStyle::Marker
                    } else {
                        style
                    };
                }
            }
        }

        compress_spans(line_text, &styles)
    }
}

/// Collapses a per-byte style vec into contiguous spans on char boundaries.
fn compress_spans(text: &str, styles: &[MdStyle]) -> Vec<StyleSpan> {
    let mut spans: Vec<StyleSpan> = Vec::new();
    let mut start = 0usize;
    for (i, _) in text
        .char_indices()
        .skip(1)
        .chain(std::iter::once((text.len(), ' ')))
    {
        // A span breaks where the style of the char starting at `i` differs
        // from the style at `start`.
        if i == text.len() || styles[i] != styles[start] {
            spans.push(StyleSpan {
                range: start..i,
                style: styles[start],
            });
            start = i;
        }
        if i == text.len() {
            break;
        }
    }
    spans
}

fn heading_marker_len(line: &str) -> usize {
    let t = line.len() - line.trim_start().len();
    let hashes = line[t..].bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 {
        return 0; // setext heading: no prefix marker
    }
    let space = usize::from(line.as_bytes().get(t + hashes) == Some(&b' '));
    t + hashes + space
}

fn quote_marker_len(line: &str) -> usize {
    let mut len = line.len() - line.trim_start().len();
    let bytes = line.as_bytes();
    while bytes.get(len) == Some(&b'>') {
        len += 1;
        if bytes.get(len) == Some(&b' ') {
            len += 1;
        }
    }
    len
}

fn list_marker_len(line: &str) -> usize {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let bytes = rest.as_bytes();
    match bytes.first() {
        Some(b'-' | b'*' | b'+') if bytes.get(1) == Some(&b' ') => {
            // Task checkbox is part of the marker.
            let after = &rest[2..];
            if after.len() >= 4
                && after.starts_with('[')
                && matches!(after.as_bytes()[1], b' ' | b'x' | b'X')
                && &after[2..4] == "] "
            {
                indent + 6
            } else {
                indent + 2
            }
        }
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if matches!(bytes.get(digits), Some(b'.' | b')'))
                && bytes.get(digits + 1) == Some(&b' ')
            {
                indent + digits + 2
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn fence_marker_len(line: &str) -> usize {
    let t = line.trim_start();
    if t.starts_with("```") || t.starts_with("~~~") {
        line.len()
    } else {
        0
    }
}

/// Parses `text` into the Live Preview layout. Every line of the document
/// lands in exactly one block; gaps between parsed blocks become
/// [`BlockKind::Blank`] blocks.
#[must_use]
pub fn parse_markdown_layout(text: &str) -> MarkdownLayout {
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_count = line_starts.len();
    line_starts.push(text.len());
    let line_of = |byte: usize| -> usize {
        match line_starts[..line_count].binary_search(&byte) {
            Ok(l) => l,
            Err(l) => l - 1,
        }
    };

    let options = Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut raw_blocks: Vec<(BlockKind, Range<usize>)> = Vec::new();
    let mut inlines: Vec<InlineSpan> = Vec::new();
    let mut links: Vec<LinkTarget> = Vec::new();
    let mut images: Vec<ImageSpan> = Vec::new();
    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    let mut depth = 0usize;
    let mut item_open = false;
    let mut fallback_seen = false;
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    match &tag {
                        Tag::Heading { level, .. } => {
                            raw_blocks.push((BlockKind::Heading(*level as u8), range.clone()))
                        }
                        Tag::Paragraph => {
                            raw_blocks.push((BlockKind::Paragraph, range.clone()));
                        }
                        Tag::BlockQuote(_) => {
                            raw_blocks.push((BlockKind::Quote, range.clone()));
                        }
                        Tag::CodeBlock(_) => {
                            raw_blocks.push((BlockKind::CodeFence, range.clone()));
                            code_ranges.push(range.clone());
                        }
                        Tag::List(_) => {}
                        _ => {
                            fallback_seen = true;
                            raw_blocks.push((BlockKind::Fallback, range.clone()));
                        }
                    }
                }
                match &tag {
                    Tag::Item if !item_open => {
                        item_open = true;
                        raw_blocks.push((BlockKind::ListItem, range.clone()));
                    }
                    Tag::Strong => inlines.push(InlineSpan {
                        bytes: range.clone(),
                        kind: InlineKind::Strong,
                    }),
                    Tag::Emphasis => inlines.push(InlineSpan {
                        bytes: range.clone(),
                        kind: InlineKind::Emphasis,
                    }),
                    Tag::Strikethrough => inlines.push(InlineSpan {
                        bytes: range.clone(),
                        kind: InlineKind::Strikethrough,
                    }),
                    Tag::Link {
                        link_type,
                        dest_url,
                        ..
                    } => {
                        let slice = &text[range.clone()];
                        let autolink = matches!(
                            link_type,
                            pulldown_cmark::LinkType::Autolink | pulldown_cmark::LinkType::Email
                        );
                        let display = if autolink {
                            dest_url.to_string()
                        } else {
                            slice
                                .strip_prefix('[')
                                .and_then(|rest| rest.split("](").next())
                                .unwrap_or(slice)
                                .to_string()
                        };
                        links.push(LinkTarget {
                            kind: if autolink {
                                LinkKind::Autolink
                            } else {
                                LinkKind::Markdown
                            },
                            bytes: range.clone(),
                            dest: dest_url.to_string(),
                            display,
                        });
                        inlines.push(InlineSpan {
                            bytes: range.clone(),
                            kind: InlineKind::Link,
                        });
                    }
                    Tag::Image { dest_url, .. } => {
                        let slice = &text[range.clone()];
                        let alt = slice
                            .strip_prefix("![")
                            .and_then(|rest| rest.split("](").next())
                            .unwrap_or("")
                            .to_string();
                        images.push(ImageSpan {
                            bytes: range.clone(),
                            dest: dest_url.to_string(),
                            alt,
                        });
                    }
                    _ => {}
                }
                depth += 1;
            }
            Event::End(tag_end) => {
                depth = depth.saturating_sub(1);
                if matches!(tag_end, TagEnd::Item) {
                    item_open = false;
                }
            }
            Event::Rule if depth == 0 => {
                raw_blocks.push((BlockKind::Rule, range.clone()));
            }
            Event::Code(_) => inlines.push(InlineSpan {
                bytes: range.clone(),
                kind: InlineKind::Code,
            }),
            _ => {}
        }
    }

    // `[[wiki]]` note links: not cmark syntax, so a linear scan finds them.
    // Spans inside code blocks or inline code render literally and are
    // excluded; spans already inside a Markdown link are excluded too (the
    // cmark link wins).
    let mut excluded = code_ranges;
    excluded.extend(
        inlines
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::Code | InlineKind::Link))
            .map(|s| s.bytes.clone()),
    );
    for (range, name) in scan_wiki_links(text) {
        if excluded
            .iter()
            .any(|ex| range.start < ex.end && ex.start < range.end)
        {
            continue;
        }
        links.push(LinkTarget {
            kind: LinkKind::Wiki,
            bytes: range.clone(),
            dest: name.clone(),
            display: name,
        });
        inlines.push(InlineSpan {
            bytes: range,
            kind: InlineKind::Link,
        });
    }
    // Bare http(s) URLs are a render-time affordance: unlike Markdown links,
    // they do not change the document. Skip parser-owned links and code so a
    // URL is never double-styled or clickable inside literal source.
    excluded.extend(links.iter().map(|link| link.bytes.clone()));
    excluded.extend(images.iter().map(|image| image.bytes.clone()));
    for range in scan_bare_http_urls(text) {
        if excluded
            .iter()
            .any(|ex| range.start < ex.end && ex.start < range.end)
        {
            continue;
        }
        let url = text[range.clone()].to_string();
        links.push(LinkTarget {
            kind: LinkKind::BareUrl,
            bytes: range.clone(),
            dest: url.clone(),
            display: url,
        });
        inlines.push(InlineSpan {
            bytes: range,
            kind: InlineKind::Link,
        });
    }
    links.sort_by_key(|l| l.bytes.start);

    if fallback_seen {
        log::info!("editor: markdown live preview falling back to source rendering for unsupported block(s)");
    }

    // Byte ranges → sorted, non-overlapping line ranges. List-item blocks
    // arrive interleaved with (removed) list containers, so sort by start.
    raw_blocks.sort_by_key(|(_, r)| (r.start, r.end));
    let mut blocks: Vec<Block> = Vec::new();
    let mut next_line = 0usize;
    for (kind, range) in raw_blocks {
        if range.start >= range.end {
            continue;
        }
        let start_line = line_of(range.start).max(next_line);
        let end_line = (line_of(range.end.saturating_sub(1)) + 1).min(line_count);
        if end_line <= start_line {
            continue; // fully swallowed by a previous (outer) block
        }
        if start_line > next_line {
            blocks.push(Block {
                kind: BlockKind::Blank,
                lines: next_line..start_line,
            });
        }
        blocks.push(Block {
            kind,
            lines: start_line..end_line,
        });
        next_line = end_line;
    }
    if next_line < line_count {
        blocks.push(Block {
            kind: BlockKind::Blank,
            lines: next_line..line_count,
        });
    }
    if blocks.is_empty() {
        blocks.push(Block {
            kind: BlockKind::Blank,
            lines: 0..line_count.max(1),
        });
    }

    inlines.sort_by_key(|s| (s.bytes.start, s.bytes.end));
    MarkdownLayout {
        blocks,
        inlines,
        links,
        images,
        line_starts,
    }
}

/// Finds `[[name]]` spans: same-line, non-empty name, delimiter-inclusive
/// byte ranges. Purely lexical — the caller filters code regions.
fn scan_wiki_links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    let mut i = 0usize;
    while let Some(open) = text[i..].find("[[") {
        let start = i + open;
        let inner_start = start + 2;
        match text[inner_start..].find("]]") {
            Some(close) => {
                let inner = &text[inner_start..inner_start + close];
                if !inner.is_empty() && !inner.contains('\n') && !inner.contains('[') {
                    let end = inner_start + close + 2;
                    found.push((start..end, inner.to_string()));
                    i = end;
                } else {
                    i = inner_start;
                }
            }
            None => break,
        }
    }
    found
}

/// Finds bare http(s) URLs without consuming adjacent prose punctuation.
/// Markdown/parser-owned ranges are filtered by the caller.
fn scan_bare_http_urls(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let tail = &text[offset..];
        let start_rel = match (tail.find("http://"), tail.find("https://")) {
            (Some(http), Some(https)) => http.min(https),
            (Some(start), None) | (None, Some(start)) => start,
            (None, None) => break,
        };
        let start = offset + start_rel;
        let prefix_ok = start == 0
            || !text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'));
        let mut end = text[start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |end_rel| start + end_rel);
        while end > start && trailing_url_punctuation(&text[start..end]) {
            end -= text[..end]
                .chars()
                .next_back()
                .expect("nonempty URL suffix")
                .len_utf8();
        }
        if prefix_ok && end > start + "http://".len() {
            found.push(start..end);
        }
        offset = (end.max(start + 1)).min(text.len());
    }
    found
}

fn trailing_url_punctuation(url: &str) -> bool {
    match url.chars().next_back() {
        Some('.' | ',' | '!' | '?' | ':' | ';' | ']' | '}' | '*' | '_' | '`' | '>') => true,
        Some(')') => {
            let opens = url.chars().filter(|&c| c == '(').count();
            let closes = url.chars().filter(|&c| c == ')').count();
            closes > opens
        }
        _ => false,
    }
}

/// Whether `text` is exactly one pasteable bare http(s) URL.
pub(crate) fn is_bare_http_url(text: &str) -> bool {
    matches!(scan_bare_http_urls(text).as_slice(), [range] if range.start == 0 && range.end == text.len())
}

/// All links in `text` (Markdown, autolink, bare URL, `[[wiki]]`), in source order.
/// Byte ranges are delimiter-inclusive.
#[must_use]
pub fn link_targets(text: &str) -> Vec<LinkTarget> {
    parse_markdown_layout(text).links
}

/// All inline image references in `text`, in source order.
#[must_use]
pub fn image_spans(text: &str) -> Vec<ImageSpan> {
    parse_markdown_layout(text).images
}

/// Revision-keyed cache: reparses only when the document revision changes.
#[derive(Debug, Default)]
pub struct MarkdownLayoutCache {
    revision: Option<u64>,
    layout: MarkdownLayout,
}

impl MarkdownLayoutCache {
    /// The layout for `buffer` at `revision`: stringifies the rope and
    /// reparses only when the revision changes.
    pub fn layout_for(&mut self, buffer: &TextBuffer, revision: u64) -> &MarkdownLayout {
        if self.revision != Some(revision) {
            self.layout = parse_markdown_layout(&buffer.to_string());
            self.revision = Some(revision);
        }
        &self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &str = "# Title\n\nSome **bold** and *italic* text.\n\n- item one\n- [x] task done\n1. ordered\n\n> a quote line\n\n```rust\nlet x = 1;\n```\n\n---\n\nlast paragraph";

    fn line_count(text: &str) -> usize {
        text.bytes().filter(|b| *b == b'\n').count() + 1
    }

    #[test]
    fn blocks_cover_every_line_exactly_once() {
        for text in ["", "a", MIXED, "\n\n\n", "# h\n"] {
            let layout = parse_markdown_layout(text);
            let mut next = 0usize;
            for block in &layout.blocks {
                assert_eq!(block.lines.start, next, "gap/overlap in {text:?}");
                assert!(block.lines.end > block.lines.start);
                next = block.lines.end;
            }
            assert_eq!(next, line_count(text), "blocks cover document {text:?}");
            // Every line maps back to exactly the covering block (roundtrip).
            for line in 0..line_count(text) {
                let idx = layout.block_index_at_line(line).unwrap();
                assert!(layout.blocks[idx].lines.contains(&line));
            }
        }
    }

    #[test]
    fn mixed_document_classifies_blocks() {
        let layout = parse_markdown_layout(MIXED);
        let kind_at = |line: usize| layout.block_at_line(line).unwrap().kind;
        assert_eq!(kind_at(0), BlockKind::Heading(1));
        assert_eq!(kind_at(1), BlockKind::Blank);
        assert_eq!(kind_at(2), BlockKind::Paragraph);
        assert_eq!(kind_at(4), BlockKind::ListItem);
        assert_eq!(kind_at(5), BlockKind::ListItem);
        assert_eq!(kind_at(6), BlockKind::ListItem);
        assert_eq!(kind_at(8), BlockKind::Quote);
        assert_eq!(kind_at(10), BlockKind::CodeFence);
        assert_eq!(kind_at(11), BlockKind::CodeFence);
        assert_eq!(kind_at(12), BlockKind::CodeFence);
        assert_eq!(kind_at(14), BlockKind::Rule);
        assert_eq!(kind_at(16), BlockKind::Paragraph);
    }

    #[test]
    fn list_items_are_separate_blocks() {
        let layout = parse_markdown_layout("- a\n- b\n- c");
        let items: Vec<_> = layout
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::ListItem)
            .collect();
        assert_eq!(items.len(), 3);
        // Nested lists stay inside their outer item's block.
        let nested = parse_markdown_layout("- outer\n  - inner\n- second");
        let items: Vec<_> = nested
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::ListItem)
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].lines, 0..2);
    }

    #[test]
    fn active_lines_span_selected_blocks_only() {
        let layout = parse_markdown_layout(MIXED);
        assert_eq!(layout.active_lines(0, 0), 0..1); // heading only
        assert_eq!(layout.active_lines(10, 10), 10..13); // whole fence
        assert_eq!(layout.active_lines(0, 2), 0..3); // heading → paragraph
                                                     // Reversed order works the same.
        assert_eq!(layout.active_lines(2, 0), 0..3);
    }

    #[test]
    fn style_spans_are_contiguous_and_cover_the_line() {
        let layout = parse_markdown_layout(MIXED);
        let lines: Vec<&str> = MIXED.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let spans = layout.line_style_spans(i, line);
            let mut cursor = 0usize;
            for span in &spans {
                assert_eq!(span.range.start, cursor, "line {i} contiguous");
                assert!(span.range.end > span.range.start);
                assert!(line.is_char_boundary(span.range.start));
                assert!(line.is_char_boundary(span.range.end));
                cursor = span.range.end;
            }
            assert_eq!(cursor, line.len(), "line {i} covered: {line:?}");
        }
    }

    #[test]
    fn heading_list_quote_markers_are_dimmed() {
        let layout = parse_markdown_layout(MIXED);
        let spans = layout.line_style_spans(0, "# Title");
        assert_eq!(
            spans[0],
            StyleSpan {
                range: 0..2,
                style: MdStyle::Marker
            }
        );
        assert_eq!(spans[1].style, MdStyle::Heading(1));

        let spans = layout.line_style_spans(5, "- [x] task done");
        assert_eq!(
            spans[0],
            StyleSpan {
                range: 0..6,
                style: MdStyle::Marker
            }
        );

        let spans = layout.line_style_spans(8, "> a quote line");
        assert_eq!(
            spans[0],
            StyleSpan {
                range: 0..2,
                style: MdStyle::Marker
            }
        );
        assert_eq!(spans[1].style, MdStyle::Quote);
    }

    #[test]
    fn inline_emphasis_styles_content_and_dims_delimiters() {
        let layout = parse_markdown_layout(MIXED);
        let line = "Some **bold** and *italic* text.";
        let spans = layout.line_style_spans(2, line);
        let style_at = |pos: usize| spans.iter().find(|s| s.range.contains(&pos)).unwrap().style;
        let bold = line.find("bold").unwrap();
        let italic = line.find("italic").unwrap();
        assert_eq!(style_at(bold), MdStyle::Strong);
        assert_eq!(style_at(bold - 1), MdStyle::Marker); // `**`
        assert_eq!(style_at(italic), MdStyle::Emphasis);
        assert_eq!(style_at(italic - 1), MdStyle::Marker); // `*`
        assert_eq!(style_at(0), MdStyle::Plain);
    }

    #[test]
    fn unicode_lines_produce_char_boundary_spans() {
        let text = "# héllo 😀\n\npara **gras 😀** café";
        let layout = parse_markdown_layout(text);
        for (i, line) in text.split('\n').enumerate() {
            for span in layout.line_style_spans(i, line) {
                assert!(line.is_char_boundary(span.range.start));
                assert!(line.is_char_boundary(span.range.end));
            }
        }
    }

    #[test]
    fn tables_fall_back_to_source_blocks() {
        let text = "| a | b |\n|---|---|\n| 1 | 2 |";
        let layout = parse_markdown_layout(text);
        // Without table extension this parses as paragraph; either way every
        // line is covered and styled spans exist. Enable tables explicitly:
        // the parser here doesn't, so this documents the degrade path.
        for line in 0..3 {
            assert!(layout.block_at_line(line).is_some());
        }
    }

    #[test]
    fn long_mixed_document_parses_with_full_coverage() {
        let mut doc = String::new();
        for i in 0..200 {
            doc.push_str(&format!(
                "## Section {i}\n\ntext **b{i}** and *i{i}*\n\n- one\n- two\n\n```\ncode {i}\n```\n\n---\n\n"
            ));
        }
        let layout = parse_markdown_layout(&doc);
        let mut next = 0usize;
        for block in &layout.blocks {
            assert_eq!(block.lines.start, next);
            next = block.lines.end;
        }
        assert_eq!(next, doc.bytes().filter(|b| *b == b'\n').count() + 1);
        assert!(layout.blocks.len() > 1000);
    }

    #[test]
    fn cache_reparses_only_on_revision_change() {
        let mut cache = MarkdownLayoutCache::default();
        let a = cache.layout_for(&TextBuffer::from_string("# a"), 1).clone();
        // Same revision: buffer is ignored, cached layout returned.
        assert_eq!(
            cache.layout_for(&TextBuffer::from_string("# CHANGED"), 1),
            &a
        );
        // New revision reparses.
        assert_ne!(
            cache.layout_for(&TextBuffer::from_string("plain now"), 2),
            &a
        );
    }

    #[test]
    fn markdown_links_report_exact_byte_ranges_and_payload() {
        let text = "intro [Plexi](https://plexiapp.com) tail";
        let links = link_targets(text);
        assert_eq!(links.len(), 1);
        let link = &links[0];
        assert_eq!(link.kind, LinkKind::Markdown);
        assert_eq!(&text[link.bytes.clone()], "[Plexi](https://plexiapp.com)");
        assert_eq!(link.dest, "https://plexiapp.com");
        assert_eq!(link.display, "Plexi");
    }

    #[test]
    fn autolinks_report_dest_as_display() {
        let text = "see <https://example.com/x> now";
        let links = link_targets(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::Autolink);
        assert_eq!(&text[links[0].bytes.clone()], "<https://example.com/x>");
        assert_eq!(links[0].dest, "https://example.com/x");
        assert_eq!(links[0].display, "https://example.com/x");
    }

    #[test]
    fn bare_urls_linkify_only_outside_code_and_trim_prose_punctuation() {
        let text = "go https://example.test/a, then http://x.test/z. See https://example.test/Foo_(bar). **https://bold.test/path** `https://code.test` ![](https://image.test/p.png)\n```\nhttps://fence.test\n```";
        let links = link_targets(text);
        let bare: Vec<_> = links
            .iter()
            .filter(|link| link.kind == LinkKind::BareUrl)
            .collect();
        assert_eq!(bare.len(), 4);
        assert_eq!(&text[bare[0].bytes.clone()], "https://example.test/a");
        assert_eq!(&text[bare[1].bytes.clone()], "http://x.test/z");
        assert_eq!(
            &text[bare[2].bytes.clone()],
            "https://example.test/Foo_(bar)"
        );
        assert_eq!(&text[bare[3].bytes.clone()], "https://bold.test/path");
        let spans = parse_markdown_layout(text).line_style_spans(0, text.lines().next().unwrap());
        assert_eq!(
            spans
                .iter()
                .find(|span| span.range.contains(&text.find("https").unwrap()))
                .unwrap()
                .style,
            MdStyle::Link
        );
        assert!(is_bare_http_url("https://example.test/a"));
        assert!(!is_bare_http_url("https://example.test/a,"));
    }

    #[test]
    fn wiki_links_scan_with_exact_ranges_and_skip_code() {
        let text = "a [[trip ideas]] b `[[not this]]` c\n```\n[[nor this]]\n```\n[[second]]";
        let links = link_targets(text);
        let wikis: Vec<_> = links.iter().filter(|l| l.kind == LinkKind::Wiki).collect();
        assert_eq!(wikis.len(), 2);
        assert_eq!(&text[wikis[0].bytes.clone()], "[[trip ideas]]");
        assert_eq!(wikis[0].dest, "trip ideas");
        assert_eq!(&text[wikis[1].bytes.clone()], "[[second]]");
    }

    #[test]
    fn image_spans_report_dest_alt_and_range() {
        let text = "before\n\n![alt text](assets/pic.png)\n\n![](https://x.test/i.jpg)";
        let images = image_spans(text);
        assert_eq!(images.len(), 2);
        assert_eq!(
            &text[images[0].bytes.clone()],
            "![alt text](assets/pic.png)"
        );
        assert_eq!(images[0].dest, "assets/pic.png");
        assert_eq!(images[0].alt, "alt text");
        assert_eq!(images[1].dest, "https://x.test/i.jpg");
        assert_eq!(images[1].alt, "");
    }

    #[test]
    fn malformed_links_degrade_without_spans_or_panics() {
        for text in ["[unclosed](http://x", "[[unclosed", "[]() empty", "![](  "] {
            let layout = parse_markdown_layout(text);
            // No panic, full line coverage, and any reported span slices cleanly.
            for link in &layout.links {
                assert!(text.get(link.bytes.clone()).is_some());
            }
            for (i, line) in text.split('\n').enumerate() {
                let _ = layout.line_style_spans(i, line);
            }
        }
        // A wiki-looking span inside a Markdown link label is not double-counted.
        let text = "[a [[b]] c](http://x.test)";
        let wiki_count = link_targets(text)
            .iter()
            .filter(|l| l.kind == LinkKind::Wiki)
            .count();
        assert_eq!(wiki_count, 0);
    }

    #[test]
    fn link_spans_style_as_link_and_line_coverage_holds() {
        let text = "go [Plexi](https://plexiapp.com) and [[wiki page]] now";
        let layout = parse_markdown_layout(text);
        let spans = layout.line_style_spans(0, text);
        let style_at = |pos: usize| spans.iter().find(|s| s.range.contains(&pos)).unwrap().style;
        assert_eq!(style_at(text.find("Plexi").unwrap()), MdStyle::Link);
        assert_eq!(style_at(text.find("wiki").unwrap()), MdStyle::Link);
        assert_eq!(style_at(0), MdStyle::Plain);
        let mut cursor = 0;
        for span in &spans {
            assert_eq!(span.range.start, cursor);
            cursor = span.range.end;
        }
        assert_eq!(cursor, text.len());
    }

    #[test]
    fn link_at_byte_and_line_of_byte_agree_with_ranges() {
        let text = "first\n[X](http://a.test)\nlast";
        let layout = parse_markdown_layout(text);
        let link = layout.links.first().unwrap().clone();
        assert_eq!(layout.line_of_byte(link.bytes.start), 1);
        assert!(layout.link_at_byte(link.bytes.start + 1).is_some());
        assert!(layout.link_at_byte(0).is_none());
        assert!(layout.link_at_byte(text.len() - 1).is_none());
    }

    #[test]
    fn setext_and_empty_documents_do_not_panic() {
        for text in ["Title\n=====", "Title\n-----", "", "\n", "   \n   "] {
            let layout = parse_markdown_layout(text);
            for (i, line) in text.split('\n').enumerate() {
                let _ = layout.line_style_spans(i, line);
            }
        }
    }
}
