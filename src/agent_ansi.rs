//! Tiny markdown-to-ANSI converter for agent mode turns.
//!
//! Just enough to make LLM replies look passable inside a terminal grid —
//! bold, italic, inline code, fenced code blocks, headers (rendered as bold).
//! No lists, no links, no tables. If we ever need more, the right move is a
//! real parser (pulldown-cmark). For now, ~60 lines is enough.
//!
//! Output is intended to be fed to `TerminalBackend::write_agent_bytes`, which
//! runs it through the same VTE parser that handles shell output.

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ITALIC: &str = "\x1b[3m";
const INLINE_CODE: &str = "\x1b[7m"; // reverse video — renders as a filled chip
const CODE_BLOCK_PREFIX: &str = "\x1b[2m\u{2502} \x1b[22m"; // dim left border

/// Convert a markdown string to ANSI-escaped text with CRLF line endings.
///
/// Lines are joined with `\r\n` because alacritty's VTE parser needs both a
/// carriage return and a line feed to advance to column 0 on the next row.
pub fn markdown_to_ansi(md: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;

    for raw_line in md.lines() {
        // Fenced code block toggle
        if raw_line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            // Skip the fence line itself; the border prefix on content lines
            // provides enough visual cue.
            continue;
        }

        if in_code_block {
            out.push_str(CODE_BLOCK_PREFIX);
            out.push_str(raw_line);
            out.push_str(RESET);
            out.push_str("\r\n");
            continue;
        }

        // Headers: leading #'s → bold the remainder of the line.
        let line = raw_line.trim_start_matches(' ');
        if let Some(rest) = line.strip_prefix("# ")
            .or_else(|| line.strip_prefix("## "))
            .or_else(|| line.strip_prefix("### "))
            .or_else(|| line.strip_prefix("#### "))
        {
            out.push_str(BOLD);
            out.push_str(rest);
            out.push_str(RESET);
            out.push_str("\r\n");
            continue;
        }

        out.push_str(&render_inline(raw_line));
        out.push_str("\r\n");
    }

    out
}

/// Inline markdown: `**bold**`, `*italic*`, and `` `code` ``.
/// Byte-level scan; marker chars are ASCII so byte indices are safe, and
/// we copy slices of the original string (UTF-8 preserved) between markers.
fn render_inline(line: &str) -> String {
    let mut out = String::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // **bold**
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(end) = find_close(&bytes[i + 2..], b"**") {
                out.push_str(BOLD);
                out.push_str(&line[i + 2..i + 2 + end]);
                out.push_str(RESET);
                i += 2 + end + 2;
                continue;
            }
        }
        // `code`
        if bytes[i] == b'`' {
            if let Some(end) = find_close(&bytes[i + 1..], b"`") {
                out.push_str(INLINE_CODE);
                out.push_str(&line[i + 1..i + 1 + end]);
                out.push_str(RESET);
                i += 1 + end + 1;
                continue;
            }
        }
        // *italic* (single asterisk, not part of **)
        if bytes[i] == b'*' {
            if let Some(end) = find_close(&bytes[i + 1..], b"*") {
                out.push_str(ITALIC);
                out.push_str(&line[i + 1..i + 1 + end]);
                out.push_str(RESET);
                i += 1 + end + 1;
                continue;
            }
        }
        // Advance by one UTF-8 char, copying the original bytes verbatim.
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_char_len(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if byte < 0xC0 {
        1 // continuation byte — shouldn't happen at char start; treat as 1
    } else if byte < 0xE0 {
        2
    } else if byte < 0xF0 {
        3
    } else {
        4
    }
}

fn find_close(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
