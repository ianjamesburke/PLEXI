//! Syntax-highlight span provider for code mode (stint 0318). No egui.
//!
//! Spans are semantic token kinds over byte ranges of a single line; color
//! mapping is the widget/theme's job, so the core stays presentation-agnostic
//! and the highlighter never names a concrete color.
//!
//! Caching: results are cached per line, keyed by the document revision.
//! Syntect parse state is sequential (block comments, fenced strings), so the
//! cache stores the parser state entering every line and extends lazily up to
//! the highest line the widget has asked for. An edit bumps the revision; the
//! cache keeps its unchanged-text prefix and reparses only from the first
//! differing line down to the last visible one — never the whole document,
//! never per frame.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

use super::buffer::TextBuffer;
use super::movement::line_text;

/// Semantic token classes the widget can map onto theme colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Punctuation,
}

/// One highlighted region: byte offsets into the line's text (always on
/// char boundaries — syntect offsets are valid UTF-8 boundaries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Anything that can produce highlight spans for a line at a document
/// revision. The widget's plain-text fallback is simply "no provider".
pub trait SpanProvider {
    /// Spans for `line`, contiguous over the whole line text. `revision`
    /// invalidates internal caches when it changes.
    fn line_spans(&mut self, buffer: &TextBuffer, line: usize, revision: u64) -> &[HighlightSpan];
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Syntect-backed [`SpanProvider`] with a revision-keyed per-line cache.
pub struct SyntaxHighlighter {
    /// Revision the cache below is valid for.
    revision: Option<u64>,
    /// Parser state entering line `i` (`states[0]` is the initial state);
    /// always `lines.len() + 1` entries.
    states: Vec<ParseState>,
    /// Scope stack entering line `i`, parallel to `states`.
    stacks: Vec<ScopeStack>,
    /// Cached spans per already-highlighted line.
    lines: Vec<Vec<HighlightSpan>>,
    /// The text each cached line was highlighted from, so a revision bump can
    /// keep the unchanged prefix instead of reparsing from the top.
    texts: Vec<String>,
}

impl SyntaxHighlighter {
    /// Builds a highlighter for `language` (an extension token or syntax
    /// name, e.g. `"rs"`, `"py"`). Returns `None` when the bundled syntax set
    /// has no match — callers fall back to plain spans.
    #[must_use]
    pub fn new(language: &str) -> Option<Self> {
        let syntax = syntax_set().find_syntax_by_token(language)?;
        log::info!(
            "editor: syntax highlighter ready for language {language:?} -> {}",
            syntax.name
        );
        Some(Self {
            revision: None,
            states: vec![ParseState::new(syntax)],
            stacks: vec![ScopeStack::new()],
            lines: Vec::new(),
            texts: Vec::new(),
        })
    }

    /// On a revision change, keeps the longest cached prefix whose line text
    /// is unchanged (identical text prefix ⇒ identical parser state) and
    /// truncates from the first differing line. Typing then reparses only
    /// from the edit down to the requested (visible) line, never the whole
    /// document prefix per keystroke.
    fn sync_revision(&mut self, buffer: &TextBuffer, revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        let keep = self
            .texts
            .iter()
            .take(buffer.line_count())
            .enumerate()
            .take_while(|(i, cached)| line_text(buffer, *i) == **cached)
            .count();
        self.texts.truncate(keep);
        self.lines.truncate(keep);
        self.states.truncate(keep + 1);
        self.stacks.truncate(keep + 1);
    }

    /// Highlights one more line, extending the cache by one entry.
    fn advance(&mut self, buffer: &TextBuffer) {
        let line = self.lines.len();
        let source = line_text(buffer, line);
        // `load_defaults_newlines` grammars expect the trailing newline.
        let text = format!("{source}\n");
        let content_len = text.len().saturating_sub(1);
        let mut state = self.states[line].clone();
        let mut stack = self.stacks[line].clone();
        let mut spans = Vec::new();
        match state.parse_line(&text, syntax_set()) {
            Ok(ops) => {
                let mut prev = 0usize;
                for (offset, op) in ops {
                    let offset = offset.min(content_len);
                    if offset > prev {
                        spans.push(HighlightSpan {
                            start: prev,
                            end: offset,
                            kind: kind_of(&stack),
                        });
                        prev = offset;
                    }
                    if let Err(e) = stack.apply(&op) {
                        log::error!("editor: highlight scope apply failed on line {line}: {e}");
                    }
                }
                if content_len > prev {
                    spans.push(HighlightSpan {
                        start: prev,
                        end: content_len,
                        kind: kind_of(&stack),
                    });
                }
            }
            Err(e) => {
                log::error!("editor: highlight parse failed on line {line}: {e}");
                if content_len > 0 {
                    spans.push(HighlightSpan {
                        start: 0,
                        end: content_len,
                        kind: TokenKind::Plain,
                    });
                }
            }
        }
        self.states.push(state);
        self.stacks.push(stack);
        self.lines.push(spans);
        self.texts.push(source);
    }
}

impl SpanProvider for SyntaxHighlighter {
    fn line_spans(&mut self, buffer: &TextBuffer, line: usize, revision: u64) -> &[HighlightSpan] {
        self.sync_revision(buffer, revision);
        let line = line.min(buffer.line_count().saturating_sub(1));
        while self.lines.len() <= line {
            self.advance(buffer);
        }
        &self.lines[line]
    }
}

/// Maps the scope stack to a token kind. Comment/string containers win over
/// anything nested inside them (so delimiters like `"` or `//` — which sit on
/// the stack as `punctuation.definition.*` — color as the container); other
/// kinds resolve innermost-first.
fn kind_of(stack: &ScopeStack) -> TokenKind {
    for scope in stack.as_slice() {
        let name = scope.build_string();
        if name.starts_with("comment") {
            return TokenKind::Comment;
        }
        if name.starts_with("string") {
            return TokenKind::String;
        }
    }
    for scope in stack.as_slice().iter().rev() {
        let name = scope.build_string();
        let kind = if name.starts_with("comment") {
            TokenKind::Comment
        } else if name.starts_with("string") {
            TokenKind::String
        } else if name.starts_with("constant") {
            TokenKind::Number
        } else if name.starts_with("keyword")
            || name.starts_with("storage.modifier")
            || name.starts_with("storage.type")
        {
            TokenKind::Keyword
        } else if name.starts_with("entity.name.function") || name.starts_with("support.function")
        {
            TokenKind::Function
        } else if name.starts_with("entity.name.type")
            || name.starts_with("entity.name")
            || name.starts_with("support.type")
            || name.starts_with("support.class")
        {
            TokenKind::Type
        } else if name.starts_with("punctuation") {
            TokenKind::Punctuation
        } else {
            continue;
        };
        return kind;
    }
    TokenKind::Plain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str) -> TextBuffer {
        TextBuffer::from_string(text)
    }

    fn kinds_at(spans: &[HighlightSpan], text: &str, needle: &str) -> TokenKind {
        let start = text.find(needle).expect("needle present");
        spans
            .iter()
            .find(|s| s.start <= start && start < s.end)
            .map(|s| s.kind)
            .expect("span covers needle")
    }

    #[test]
    fn unknown_language_yields_no_highlighter() {
        assert!(SyntaxHighlighter::new("definitely-not-a-language").is_none());
    }

    #[test]
    fn rust_keywords_strings_and_comments_are_classified() {
        let text = "fn main() { let s = \"hi\"; } // done";
        let buf = buffer(text);
        let mut h = SyntaxHighlighter::new("rs").expect("rust syntax bundled");
        let spans = h.line_spans(&buf, 0, 0).to_vec();
        assert_eq!(kinds_at(&spans, text, "fn"), TokenKind::Keyword);
        assert_eq!(kinds_at(&spans, text, "let"), TokenKind::Keyword);
        assert_eq!(kinds_at(&spans, text, "\"hi\""), TokenKind::String);
        assert_eq!(kinds_at(&spans, text, "// done"), TokenKind::Comment);
    }

    #[test]
    fn spans_are_contiguous_and_on_char_boundaries() {
        let text = "let s = \"héllo 😀\"; // café";
        let buf = buffer(text);
        let mut h = SyntaxHighlighter::new("rs").unwrap();
        let spans = h.line_spans(&buf, 0, 0);
        let mut cursor = 0usize;
        for span in spans {
            assert_eq!(span.start, cursor, "spans are contiguous");
            assert!(span.end > span.start);
            assert!(text.is_char_boundary(span.start));
            assert!(text.is_char_boundary(span.end));
            cursor = span.end;
        }
        assert_eq!(cursor, text.len(), "spans cover the whole line");
    }

    #[test]
    fn parser_state_carries_across_lines_in_block_comments() {
        let buf = buffer("/*\ninside comment\n*/\nfn after() {}");
        let mut h = SyntaxHighlighter::new("rs").unwrap();
        let spans = h.line_spans(&buf, 1, 0).to_vec();
        assert!(
            spans.iter().all(|s| s.kind == TokenKind::Comment),
            "line inside a block comment is all comment: {spans:?}"
        );
        let after = h.line_spans(&buf, 3, 0).to_vec();
        assert_eq!(kinds_at(&after, "fn after() {}", "fn"), TokenKind::Keyword);
    }

    #[test]
    fn revision_change_invalidates_the_cache() {
        let before = buffer("let x = 1;");
        let mut h = SyntaxHighlighter::new("rs").unwrap();
        let first = h.line_spans(&before, 0, 0).to_vec();
        assert_eq!(kinds_at(&first, "let x = 1;", "let"), TokenKind::Keyword);

        // Same revision: cached (identity of contents).
        assert_eq!(h.line_spans(&before, 0, 0), &first[..]);

        // New revision with different text: recomputed against the new buffer.
        let after = buffer("// now a comment");
        let second = h.line_spans(&after, 0, 1).to_vec();
        assert_eq!(
            kinds_at(&second, "// now a comment", "//"),
            TokenKind::Comment
        );
    }

    #[test]
    fn revision_bump_keeps_unchanged_prefix_and_reparses_the_suffix() {
        let before = buffer("fn a() {}\nlet x = 1;\nlet y = 2;");
        let mut h = SyntaxHighlighter::new("rs").unwrap();
        h.line_spans(&before, 2, 0);
        assert_eq!(h.lines.len(), 3);

        // Edit only line 2: the two-line prefix cache survives the revision
        // bump (a request for line 0 triggers the sync but no reparse of the
        // kept prefix — only the changed suffix is dropped).
        let after = buffer("fn a() {}\nlet x = 1;\n// changed");
        h.line_spans(&after, 0, 1);
        assert_eq!(h.lines.len(), 2, "unchanged prefix kept, suffix dropped");
        let spans = h.line_spans(&after, 2, 1).to_vec();
        assert_eq!(kinds_at(&spans, "// changed", "//"), TokenKind::Comment);

        // An upstream edit that changes downstream state truncates from the
        // edit: opening a block comment on line 0 re-classifies line 1.
        let commented = buffer("/* start\nlet x = 1;\n// changed");
        let spans = h.line_spans(&commented, 1, 2).to_vec();
        assert!(spans.iter().all(|s| s.kind == TokenKind::Comment), "{spans:?}");
    }

    #[test]
    fn empty_lines_produce_no_spans() {
        let buf = buffer("fn a() {}\n\nfn b() {}");
        let mut h = SyntaxHighlighter::new("rs").unwrap();
        assert!(h.line_spans(&buf, 1, 0).is_empty());
    }
}
