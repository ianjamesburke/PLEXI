//! Text a plugin writes, made safe for host surfaces.
//!
//! A plugin is untrusted. Its title, `log` messages, stderr lines and fatal
//! error reach the pane label, `plexi.log` and terminals reading that log, so
//! every such path goes through here: control and bidi characters become
//! visible escapes, and the text is capped with a marker (stint 1344).

/// Marks text cut at its cap.
pub(super) const TRUNCATED: &str = " [truncated]";
/// One guest log message or stderr line.
pub(super) const MAX_LINE_CHARS: usize = 2048;
/// A pane title.
pub(super) const MAX_TITLE_CHARS: usize = 128;

/// Characters that move the cursor, start a new line, recolor a terminal or
/// reorder the text around them.
pub(super) fn is_unsafe_to_print(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Replace every unprintable character with its visible escape (`\n`,
/// `\u{1b}`), so the text occupies exactly the lines the host gives it.
pub(super) fn escape_untrusted(text: &str, keep_newlines: bool) -> String {
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

/// At most `max_chars` of `text`, marked when cut.
pub(super) fn keep_head(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}{TRUNCATED}", &text[..cut]),
        None => text.to_string(),
    }
}

/// One line of guest text for a host surface: capped at `max_chars`, then
/// escaped. Truncation leaves an info trace naming the plugin and `sink`.
pub(super) fn guest_line(plugin_id: &str, sink: &str, text: &str, max_chars: usize) -> String {
    let chars = text.chars().count();
    if chars > max_chars {
        log::info!("plugin::{plugin_id}: guest {sink} truncated from {chars} to {max_chars} chars");
    }
    escape_untrusted(&keep_head(text, max_chars), false)
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
    }

    #[test]
    fn short_clean_text_is_unchanged() {
        assert_eq!(
            guest_line("probe", "title", "Calc", MAX_TITLE_CHARS),
            "Calc"
        );
    }
}
