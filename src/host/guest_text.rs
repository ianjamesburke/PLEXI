//! Text a plugin writes, made safe for host surfaces.
//!
//! A plugin is untrusted. Its title, log messages, stderr, errors,
//! notifications and the identifiers it names reach the pane label,
//! `plexi.log` and terminals reading that log, so every such path goes
//! through here (stint 1344): characters that move the cursor, start a line,
//! recolor a terminal, reorder text or hide inside it become visible escapes,
//! and the text is capped with a marker.
//!
//! The cap counts characters *before* escaping, so escaped output can be
//! longer than the cap: each escaped character becomes at most 10 (`\u{e007f}`),
//! so a capped line is at most `10 × cap + TRUNCATED` characters long.

/// Marks text cut at its cap.
pub(crate) const TRUNCATED: &str = " [truncated]";
/// Marks a block whose earlier lines were dropped to keep its end. Blocks are
/// tracebacks or the stderr that carries one, so the marker says so (the
/// wording `fatal_error` reports have used since PR #3090).
pub(crate) const BLOCK_TRUNCATED: &str = "[traceback truncated: earlier frames dropped]";
/// One log message, stderr line, error or notification body.
pub(crate) const MAX_LINE_CHARS: usize = 2048;
/// A pane or notification title.
pub(crate) const MAX_TITLE_CHARS: usize = 128;
/// An identifier a plugin names (request, handler, server, event, capability).
pub(crate) const MAX_ID_CHARS: usize = 256;
/// A multi-line block (traceback, captured stderr): keeps its end.
pub(crate) const MAX_BLOCK_BYTES: usize = 64 * 1024;
/// One host log record is often written per block line, so lines are capped too.
pub(crate) const MAX_BLOCK_LINES: usize = 200;

/// Characters that move the cursor, start a new line, recolor a terminal,
/// reorder the text around them, or are invisible and let one string pass for
/// another. The zero-width joiner (U+200D) is kept: emoji sequences need it.
pub(crate) fn is_unsafe_to_print(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// Replace every unsafe character with its visible escape (`\n`, `\u{1b}`),
/// so the text occupies exactly the lines the host gives it.
pub(crate) fn escape_untrusted(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_unsafe_to_print(c) && !(keep_newlines && c == '\n') {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// At most `max_chars` of `text`, marked when cut. `Some(original count)`
/// when it was cut.
fn keep_head(text: &str, max_chars: usize) -> (String, Option<usize>) {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => (
            format!("{}{TRUNCATED}", &text[..cut]),
            Some(text.chars().count()),
        ),
        None => (text.to_string(), None),
    }
}

/// The end of `text`: at most `MAX_BLOCK_BYTES`, cut forward to a whole line,
/// and at most `MAX_BLOCK_LINES` lines. A cut is marked on its own first line.
fn keep_tail(text: &str) -> (String, bool) {
    let text = text.trim_end();
    let mut tail = text;
    if tail.len() > MAX_BLOCK_BYTES {
        let mut cut = tail.len() - MAX_BLOCK_BYTES;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail = &tail[cut..];
        tail = tail.find('\n').map_or(tail, |newline| &tail[newline + 1..]);
    }
    if let Some((cut, _)) = tail.rmatch_indices('\n').nth(MAX_BLOCK_LINES - 1) {
        tail = &tail[cut + 1..];
    }
    if tail.len() == text.len() {
        (text.to_string(), false)
    } else {
        (format!("{BLOCK_TRUNCATED}\n{tail}"), true)
    }
}

/// The one truncation trace, so every sink reports a cut the same way.
pub(crate) fn trace_truncation(plugin_id: &str, sink: &str, from: usize, to: usize) {
    log::info!(
        "plugin::{}: guest {sink} truncated from {from} to {to}",
        escape_untrusted(plugin_id, false)
    );
}

/// One line of guest text, capped and escaped, without a trace. For callers
/// that trace once per event themselves (`guest_line` traces every call).
pub(crate) fn bound_line(text: &str, max_chars: usize) -> (String, Option<usize>) {
    let (head, cut_from) = keep_head(text, max_chars);
    (escape_untrusted(&head, false), cut_from)
}

/// One line of guest text for a host surface: capped at `max_chars`, then
/// escaped. Truncation leaves an info trace naming the plugin and `sink`.
pub(crate) fn guest_line(plugin_id: &str, sink: &str, text: &str, max_chars: usize) -> String {
    let (line, cut_from) = bound_line(text, max_chars);
    if let Some(from) = cut_from {
        trace_truncation(plugin_id, sink, from, max_chars);
    }
    line
}

/// Guest text that may keep its line breaks (a notification body): capped at
/// `max_chars`, every other unsafe character escaped.
pub(crate) fn guest_paragraph(plugin_id: &str, sink: &str, text: &str, max_chars: usize) -> String {
    let (head, cut_from) = keep_head(text, max_chars);
    if let Some(from) = cut_from {
        trace_truncation(plugin_id, sink, from, max_chars);
    }
    escape_untrusted(&head, true)
}

/// A multi-line guest block (traceback, captured stderr): its end is kept,
/// lines stay lines, every other unsafe character is escaped.
pub(crate) fn guest_block(plugin_id: &str, sink: &str, text: &str) -> String {
    let (tail, cut) = keep_tail(text);
    if cut {
        trace_truncation(plugin_id, sink, text.len(), MAX_BLOCK_BYTES);
    }
    escape_untrusted(tail.trim_end(), true)
}

/// A guest-named identifier, for a log line or message: escaped and capped.
/// The value itself is left as the guest sent it wherever it is a key that
/// travels back to the guest; only what is printed is made safe.
pub(crate) fn shown(text: &str) -> String {
    bound_line(text, MAX_ID_CHARS).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_line_escapes_and_caps() {
        let hostile = format!("\u{1b}[31mred\nforged\u{202e}{}", "A".repeat(3000));
        let line = guest_line("probe", "log", &hostile, MAX_LINE_CHARS);
        assert!(!line.chars().any(is_unsafe_to_print), "{line:.60?}");
        assert!(
            line.starts_with("\\u{1b}[31mred\\nforged\\u{202e}"),
            "{line:.60?}"
        );
        assert!(line.ends_with(TRUNCATED));
        assert_eq!(line.chars().count(), MAX_LINE_CHARS + 13 + TRUNCATED.len());
    }

    #[test]
    fn invisible_lookalike_characters_are_escaped() {
        for c in [
            '\u{200B}',
            '\u{2060}',
            '\u{FEFF}',
            '\u{00AD}',
            '\u{2061}',
            '\u{206A}',
            '\u{E0041}',
            '\u{FFF9}',
        ] {
            let shown = shown(&format!("a{c}b"));
            assert!(!shown.contains(c), "{c:?} survived: {shown:?}");
        }
    }

    /// Guard: text a person writes in any script comes through unchanged,
    /// including an emoji sequence held together by the zero-width joiner.
    #[test]
    fn short_clean_text_is_unchanged() {
        for text in [
            "Calc",
            "Café déjà vu",
            "日本語のタイトル",
            "👩\u{200D}💻 dev",
        ] {
            assert_eq!(guest_line("probe", "title", text, MAX_TITLE_CHARS), text);
        }
    }

    #[test]
    fn a_block_keeps_its_lines_and_its_end() {
        let block = format!("{}last\u{1b}line", "x\n".repeat(MAX_BLOCK_LINES + 5));
        let kept = guest_block("probe", "stderr", &block);
        assert!(kept.starts_with(BLOCK_TRUNCATED), "{kept:.40?}");
        assert!(kept.ends_with("last\\u{1b}line"), "{kept:?}");
        assert!(kept.lines().count() <= MAX_BLOCK_LINES + 1);
    }

    #[test]
    fn a_paragraph_keeps_newlines_only() {
        assert_eq!(
            guest_paragraph("probe", "body", "one\ntwo\u{1b}[0m", MAX_LINE_CHARS),
            "one\ntwo\\u{1b}[0m"
        );
    }
}
