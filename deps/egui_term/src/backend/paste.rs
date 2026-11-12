//! Clipboard paste encoding for the PTY.
//!
//! Modeled on Ghostty `src/input/paste.zig` and Alacritty's `paste()`:
//! when the foreground program enabled bracketed paste (`CSI ? 2004 h`) the
//! text is framed with `ESC[200~` / `ESC[201~` so the program can tell a paste
//! from typing; otherwise line breaks become the CR the Enter key produces.

pub(super) const BRACKETED_PREFIX: &[u8] = b"\x1b[200~";
pub(super) const BRACKETED_SUFFIX: &[u8] = b"\x1b[201~";

/// Bytes that must never be pasted into a non-bracketed stream: the xterm
/// text-insertion strip set plus the default tty line-discipline control
/// characters (VINTR, VQUIT, VKILL, VSUSP, VSTART, VSTOP, VWERASE, VLNEXT,
/// VREPRINT, VDISCARD), as listed by Ghostty. `\t`, `\r` and `\n` are not here.
const UNSAFE_PLAIN: &[u8] = &[
    0x00, 0x04, 0x05, 0x08, 0x1b, 0x7f, 0x03, 0x1c, 0x15, 0x1a, 0x11, 0x13,
    0x17, 0x16, 0x12, 0x0f,
];

/// Encode `text` for a single contiguous write to the PTY.
///
/// Bracketed: prefix, payload, suffix. `ESC` and `ETX` are removed from the
/// payload so it cannot forge the end marker (`ESC[201~`) and so shells that
/// treat `^C` as a terminator are not tricked (Alacritty does the same).
///
/// Plain: `\r\n` and `\n` become one `\r`; unsafe control bytes are removed.
pub(super) fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut out = Vec::with_capacity(
            BRACKETED_PREFIX.len() + text.len() + BRACKETED_SUFFIX.len(),
        );
        out.extend_from_slice(BRACKETED_PREFIX);
        out.extend(text.bytes().filter(|b| !matches!(b, 0x1b | 0x03)));
        out.extend_from_slice(BRACKETED_SUFFIX);
        return out;
    }
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\r' if bytes.peek() == Some(&b'\n') => {},
            b'\n' => out.push(b'\r'),
            b if UNSAFE_PLAIN.contains(&b) => {},
            b => out.push(b),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode_paste;

    #[test]
    fn bracketed_wraps_and_strips_esc_etx() {
        assert_eq!(
            encode_paste("a\nb\x1b[201~c\x03", true),
            b"\x1b[200~a\nb[201~c\x1b[201~".to_vec()
        );
    }

    #[test]
    fn bracketed_empty_is_just_the_frame() {
        assert_eq!(encode_paste("", true), b"\x1b[200~\x1b[201~".to_vec());
    }

    #[test]
    fn plain_converts_line_breaks_and_strips_unsafe_bytes() {
        assert_eq!(
            encode_paste("a\r\nb\nc\rd\te\x1b\x03\x7f", false),
            b"a\rb\rc\rd\te".to_vec()
        );
    }

    #[test]
    fn plain_preserves_utf8() {
        assert_eq!(
            encode_paste("héllo\n日本", false),
            "héllo\r日本".as_bytes()
        );
    }
}
