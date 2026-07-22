//! Markdown-aware editing planners: pure functions from `(buffer, selection)`
//! to a [`MarkdownPlan`] (edit operations + resulting selection). No mutation
//! happens here — [`crate::editor::Document`] commits every plan as one
//! recorded transaction, so each Markdown command is atomically undoable.
//!
//! Rules (see `docs/notes-editor.md`, Editing Contract):
//! - Tab indents the current line or every selected line (list or not);
//!   inside a fenced code block a collapsed Tab falls back to a plain
//!   caret-position indent.
//! - Enter continues unordered lists (`-`, `*`, `+`), task lists (`- [ ]`),
//!   ordered lists (renumbering the following same-indent siblings), and
//!   block quotes (`>`); Enter on an empty continuation removes the marker.
//! - Backspace on an empty marker line removes the marker, keeping the
//!   indentation (the plain smart backspace then removes indent levels).
//! - No Markdown behavior inside fenced code blocks; a planner returning
//!   `None` means "use the plain (non-Markdown) behavior".

use super::buffer::TextBuffer;
use super::cursor::{Cursor, Selection};
use super::movement;
use super::transaction::EditOperation;

/// One planned Markdown edit: operations to apply in order plus the selection
/// to leave in place. Committed atomically by the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPlan {
    pub ops: Vec<EditOperation>,
    pub selection_after: Selection,
}

/// A parsed Markdown line prefix: leading whitespace plus a structural
/// marker. All marker text is ASCII, so byte and char lengths coincide.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    /// Leading spaces/tabs.
    indent: String,
    /// The literal marker text including its trailing space(s).
    marker: String,
    /// The marker a continuation line receives (task boxes reset to `[ ]`,
    /// ordered markers increment).
    next: String,
    /// The number of an ordered-list marker.
    ordered: Option<u64>,
}

fn parse_marker(line: &str) -> Option<Marker> {
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let rest = &line[indent.len()..];
    let mut first = rest.chars();
    match first.next()? {
        '>' => {
            // Block quote, possibly nested: repeated `>` each optionally
            // followed by one space.
            let mut marker = String::new();
            let mut chars = rest.chars().peekable();
            while chars.peek() == Some(&'>') {
                chars.next();
                marker.push('>');
                if chars.peek() == Some(&' ') {
                    chars.next();
                    marker.push(' ');
                }
            }
            let next = marker.clone();
            Some(Marker {
                indent,
                marker,
                next,
                ordered: None,
            })
        }
        bullet @ ('-' | '*' | '+') if first.next() == Some(' ') => {
            let after = &rest[2..];
            let task = after.len() >= 4
                && after.starts_with('[')
                && matches!(after.as_bytes()[1], b' ' | b'x' | b'X')
                && &after[2..4] == "] ";
            let marker = if task {
                rest[..6].to_string()
            } else {
                rest[..2].to_string()
            };
            let next = if task {
                format!("{bullet} [ ] ")
            } else {
                marker.clone()
            };
            Some(Marker {
                indent,
                marker,
                next,
                ordered: None,
            })
        }
        c if c.is_ascii_digit() => {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if digits.len() > 9 {
                return None;
            }
            let after = &rest[digits.len()..];
            let delim = after.chars().next().filter(|d| *d == '.' || *d == ')')?;
            if after.chars().nth(1) != Some(' ') {
                return None;
            }
            let n: u64 = digits.parse().ok()?;
            Some(Marker {
                indent,
                marker: format!("{digits}{delim} "),
                next: format!("{}{delim} ", n + 1),
                ordered: Some(n),
            })
        }
        _ => None,
    }
}

/// A fence delimiter run: `(fence char, run length, rest-of-line is blank)`.
/// `None` when the line is not a fence delimiter.
fn fence_run(line: &str) -> Option<(char, usize, bool)> {
    let t = line.trim_start();
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = t.chars().take_while(|x| *x == c).count();
    if run < 3 {
        return None;
    }
    Some((c, run, t[run..].trim().is_empty()))
}

/// Whether `line` is inside a fenced code block. CommonMark closer rules: a
/// fence only closes on the same character, a run at least as long as the
/// opener, and no trailing info string.
fn in_fence(buffer: &TextBuffer, line: usize) -> bool {
    let mut open: Option<(char, usize)> = None;
    for l in 0..line {
        let text = movement::line_text(buffer, l);
        match (open, fence_run(&text)) {
            (None, Some((c, run, _))) => open = Some((c, run)),
            (Some((oc, olen)), Some((c, run, blank))) if c == oc && run >= olen && blank => {
                open = None;
            }
            _ => {}
        }
    }
    open.is_some()
}

/// The inclusive line range a line-wise command operates on. A multi-line
/// selection ending at column 0 of a later line excludes that line (platform
/// convention, mirrors `Document::indent_line_range`).
fn line_range(selection: Selection) -> (usize, usize) {
    let (start, end) = selection.ordered();
    let last = if end.line > start.line && end.column == 0 {
        end.line - 1
    } else {
        end.line
    };
    (start.line, last)
}

/// Tab: indent the current line, or every selected line, by one level as one
/// transaction. Lines already indented with tabs get a tab, others four
/// spaces. Returns `None` for a collapsed caret inside a fenced code block —
/// there Tab is a plain caret-position indent, not a line indent.
#[must_use]
pub fn plan_indent(buffer: &TextBuffer, selection: Selection) -> Option<MarkdownPlan> {
    let head = movement::clamp(buffer, selection.head);
    let anchor = movement::clamp(buffer, selection.anchor);
    let selection = Selection::new(anchor, head);
    if !selection.is_range() && in_fence(buffer, head.line) {
        return None;
    }
    let (first, last) = line_range(selection);
    let mut ops = Vec::new();
    let mut added = vec![0usize; last - first + 1];
    for line in (first..=last).rev() {
        let text = movement::line_text(buffer, line);
        let unit = if text.starts_with('\t') { "\t" } else { "    " };
        ops.push(EditOperation::Insert {
            pos: buffer.line_to_char(line),
            text: unit.to_string(),
        });
        added[line - first] = unit.chars().count();
    }
    let shift = |c: Cursor| {
        if c.line >= first && c.line <= last {
            Cursor::new(c.line, c.column + added[c.line - first])
        } else {
            c
        }
    };
    Some(MarkdownPlan {
        ops,
        selection_after: Selection::new(shift(anchor), shift(head)),
    })
}

/// Enter on a list/quote line: continue the structure, or exit it when the
/// line is an empty continuation. `None` means plain newline behavior
/// (selections, fenced code blocks, non-marker lines, caret inside the
/// marker prefix).
#[must_use]
pub fn plan_newline(buffer: &TextBuffer, selection: Selection) -> Option<MarkdownPlan> {
    if selection.is_range() {
        return None;
    }
    let caret = movement::clamp(buffer, selection.head);
    if in_fence(buffer, caret.line) {
        return None;
    }
    let line = movement::line_text(buffer, caret.line);
    let marker = parse_marker(&line)?;
    let prefix_chars = marker.indent.chars().count() + marker.marker.chars().count();
    if caret.column < prefix_chars {
        return None;
    }
    let content = &line[marker.indent.len() + marker.marker.len()..];
    if content.is_empty() {
        // Empty continuation: remove the whole prefix and exit the structure.
        // Following ordered siblings close the gap the removed item leaves.
        let start = buffer.line_to_char(caret.line);
        let prefix = format!("{}{}", marker.indent, marker.marker);
        let deleted = prefix.chars().count();
        let mut ops = vec![EditOperation::Delete { pos: start, text: prefix }];
        if let Some(n) = marker.ordered {
            renumber_following(
                buffer,
                caret.line,
                &marker.indent,
                n,
                -(deleted as isize),
                &mut ops,
            );
        }
        return Some(MarkdownPlan {
            ops,
            selection_after: Selection::collapsed(Cursor::new(caret.line, 0)),
        });
    }
    let caret_char = movement::cursor_to_char(buffer, caret);
    let inserted = format!("\n{}{}", marker.indent, marker.next);
    let mut ops = vec![EditOperation::Insert {
        pos: caret_char,
        text: inserted.clone(),
    }];
    // Splitting mid-item: the spaces right after the caret would otherwise
    // land between the new marker and the carried content — consume them.
    let gap: usize = line
        .chars()
        .skip(caret.column)
        .take_while(|c| *c == ' ')
        .count();
    if gap > 0 {
        ops.push(EditOperation::Delete {
            pos: caret_char + inserted.chars().count(),
            text: " ".repeat(gap),
        });
    }
    if let Some(n) = marker.ordered {
        let offset = inserted.chars().count() as isize - gap as isize;
        renumber_following(buffer, caret.line, &marker.indent, n + 2, offset, &mut ops);
    }
    let column = marker.indent.chars().count() + marker.next.chars().count();
    Some(MarkdownPlan {
        ops,
        selection_after: Selection::collapsed(Cursor::new(caret.line + 1, column)),
    })
}

/// Renumber consecutive ordered-list lines with the same indent below
/// `line`, so the run stays sequential starting at `first_expected`.
/// `initial_offset` is the net char delta of the ops already queued ahead.
fn renumber_following(
    buffer: &TextBuffer,
    line: usize,
    indent: &str,
    first_expected: u64,
    initial_offset: isize,
    ops: &mut Vec<EditOperation>,
) {
    let mut offset = initial_offset;
    let mut expected = first_expected;
    for l in (line + 1)..buffer.line_count() {
        let text = movement::line_text(buffer, l);
        let Some(m) = parse_marker(&text) else { break };
        let Some(n) = m.ordered else { break };
        if m.indent != indent {
            break;
        }
        if n != expected {
            let old_digits: String = m
                .marker
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let new_digits = expected.to_string();
            let pos = (buffer.line_to_char(l) + indent.chars().count()) as isize + offset;
            let pos = usize::try_from(pos).unwrap_or(0);
            ops.push(EditOperation::Delete {
                pos,
                text: old_digits.clone(),
            });
            ops.push(EditOperation::Insert {
                pos,
                text: new_digits.clone(),
            });
            offset += new_digits.chars().count() as isize - old_digits.chars().count() as isize;
        }
        expected += 1;
    }
}

/// Backspace with the caret at the end of an empty marker line: remove the
/// marker, keep the indentation. `None` means plain deletion (which already
/// includes smart indentation backspace).
#[must_use]
pub fn plan_backspace(buffer: &TextBuffer, selection: Selection) -> Option<MarkdownPlan> {
    if selection.is_range() {
        return None;
    }
    let caret = movement::clamp(buffer, selection.head);
    if in_fence(buffer, caret.line) {
        return None;
    }
    let line = movement::line_text(buffer, caret.line);
    let marker = parse_marker(&line)?;
    let indent_chars = marker.indent.chars().count();
    let prefix_chars = indent_chars + marker.marker.chars().count();
    if caret.column != prefix_chars || line.chars().count() != prefix_chars {
        return None;
    }
    Some(MarkdownPlan {
        ops: vec![EditOperation::Delete {
            pos: buffer.line_to_char(caret.line) + indent_chars,
            text: marker.marker,
        }],
        selection_after: Selection::collapsed(Cursor::new(caret.line, indent_chars)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(line: &str) -> Option<(String, String, String, Option<u64>)> {
        parse_marker(line).map(|m| (m.indent, m.marker, m.next, m.ordered))
    }

    #[test]
    fn parses_bullet_task_ordered_and_quote_markers() {
        assert_eq!(
            marker("- item"),
            Some((String::new(), "- ".into(), "- ".into(), None))
        );
        assert_eq!(
            marker("  * item"),
            Some(("  ".into(), "* ".into(), "* ".into(), None))
        );
        assert_eq!(
            marker("\t+ item"),
            Some(("\t".into(), "+ ".into(), "+ ".into(), None))
        );
        assert_eq!(
            marker("- [x] done"),
            Some((String::new(), "- [x] ".into(), "- [ ] ".into(), None))
        );
        assert_eq!(
            marker("3. third"),
            Some((String::new(), "3. ".into(), "4. ".into(), Some(3)))
        );
        assert_eq!(
            marker("12) x"),
            Some((String::new(), "12) ".into(), "13) ".into(), Some(12)))
        );
        assert_eq!(
            marker("> quoted"),
            Some((String::new(), "> ".into(), "> ".into(), None))
        );
        assert_eq!(
            marker("> > nested"),
            Some((String::new(), "> > ".into(), "> > ".into(), None))
        );
        // Non-markers.
        assert_eq!(marker("plain"), None);
        assert_eq!(marker("-tight"), None);
        assert_eq!(marker("1.no-space"), None);
        assert_eq!(marker(""), None);
    }

    #[test]
    fn fence_detection_toggles_on_delimiters() {
        let b = TextBuffer::from_string("text\n```rust\ncode\n```\nafter");
        assert!(!in_fence(&b, 0));
        assert!(!in_fence(&b, 1)); // the opening fence line itself
        assert!(in_fence(&b, 2));
        assert!(in_fence(&b, 3)); // closing fence line counts as inside
        assert!(!in_fence(&b, 4));
    }

    #[test]
    fn fence_closers_must_match_opener_char_and_length() {
        // A `~~~` line inside a backtick fence does not close it.
        let b = TextBuffer::from_string("```\n~~~\nstill code\n```\nafter");
        assert!(in_fence(&b, 2));
        assert!(!in_fence(&b, 4));
        // A shorter run or an info string does not close the fence.
        let b = TextBuffer::from_string("````\n```\n``` rust\nstill code\n````\nafter");
        assert!(in_fence(&b, 3));
        assert!(!in_fence(&b, 5));
    }
}
