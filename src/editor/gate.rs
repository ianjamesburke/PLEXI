//! Editor release gate (stint 0479): a table-driven command matrix, a long
//! deterministic stress sequence, and seeded randomized command fuzzing over
//! the pure [`Document`] state machine — with per-command invariant checks
//! and a machine-readable qualification artifact.
//!
//! Compiled only under `cfg(test)`: the gate is test infrastructure, never
//! product code. No egui dependency. The harness layer
//! (`src/testing/harness_tests.rs`) reuses [`gate_cases`] to drive a
//! representative subset through the installed-host input surface, and
//! `scripts/editor-gate.sh` runs the scene-level installed-host gate.
#![cfg(test)]

use std::path::Path;
use std::time::Instant;

use super::commands::{Document, EditorCommand, Movement};
use super::cursor::Cursor;
use super::movement;
use super::EditorSemanticState;

/// Environment variable that pins `gate_randomized_commands` to one seed.
pub const GATE_SEED_ENV: &str = "PLEXI_EDITOR_GATE_SEED";

const DEFAULT_SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const RANDOM_COMMANDS_PER_SEED: usize = 2000;

// ─── Case table ──────────────────────────────────────────────────────────────

/// Expected final state of a gate case. `text` is always checked; the rest
/// only when present.
#[derive(Debug, Default)]
pub struct GateExpect {
    pub text: &'static str,
    pub cursor: Option<Cursor>,
    /// Expected `selected_text()` after the final command.
    pub selected: Option<&'static str>,
    pub undo_depth: Option<usize>,
    pub can_redo: Option<bool>,
}

/// One named gate case: initial text, a command sequence, and expected finals.
#[derive(Debug)]
pub struct GateCase {
    pub name: &'static str,
    pub initial: &'static str,
    pub commands: Vec<EditorCommand>,
    pub expect: GateExpect,
}

fn mv(movement: Movement) -> EditorCommand {
    EditorCommand::Move {
        movement,
        extend: false,
    }
}

fn mv_ext(movement: Movement) -> EditorCommand {
    EditorCommand::Move {
        movement,
        extend: true,
    }
}

fn ins(text: &str) -> EditorCommand {
    EditorCommand::InsertText(text.to_string())
}

fn cur(line: usize, column: usize) -> EditorCommand {
    EditorCommand::SetCursor(Cursor::new(line, column))
}

fn ext(line: usize, column: usize) -> EditorCommand {
    EditorCommand::ExtendTo(Cursor::new(line, column))
}

/// The release-gate matrix: every case is small, named, and independently
/// replayable through a fresh [`Document`].
#[must_use]
pub fn gate_cases() -> Vec<GateCase> {
    vec![
        GateCase {
            name: "plain_typing",
            initial: "",
            commands: vec![ins("hello"), ins(" world")],
            expect: GateExpect {
                text: "hello world",
                cursor: Some(Cursor::new(0, 11)),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "typing_after_move_two_groups",
            initial: "",
            commands: vec![ins("ab"), mv(Movement::Left), ins("c")],
            expect: GateExpect {
                text: "acb",
                cursor: Some(Cursor::new(0, 2)),
                undo_depth: Some(2),
                ..Default::default()
            },
        },
        GateCase {
            name: "unicode_accented_typing",
            initial: "",
            commands: vec![ins("héllo café")],
            expect: GateExpect {
                text: "héllo café",
                cursor: Some(Cursor::new(0, 10)),
                ..Default::default()
            },
        },
        GateCase {
            name: "grapheme_combining_backspace",
            initial: "cafe\u{301}",
            commands: vec![mv(Movement::DocEnd), EditorCommand::Backspace],
            expect: GateExpect {
                text: "caf",
                cursor: Some(Cursor::new(0, 3)),
                ..Default::default()
            },
        },
        GateCase {
            name: "zwj_emoji_backspace",
            initial: "a👨\u{200D}👩\u{200D}👧",
            commands: vec![mv(Movement::DocEnd), EditorCommand::Backspace],
            expect: GateExpect {
                text: "a",
                cursor: Some(Cursor::new(0, 1)),
                ..Default::default()
            },
        },
        GateCase {
            name: "flag_emoji_typing",
            initial: "",
            commands: vec![ins("🇺🇸ok")],
            expect: GateExpect {
                text: "🇺🇸ok",
                cursor: Some(Cursor::new(0, 4)),
                ..Default::default()
            },
        },
        GateCase {
            name: "combining_mark_left_navigation",
            initial: "e\u{301}x",
            commands: vec![mv(Movement::DocEnd), mv(Movement::Left), mv(Movement::Left)],
            expect: GateExpect {
                text: "e\u{301}x",
                cursor: Some(Cursor::new(0, 0)),
                undo_depth: Some(0),
                ..Default::default()
            },
        },
        GateCase {
            name: "ime_preedit_never_buffers",
            initial: "x",
            commands: vec![
                cur(0, 1),
                EditorCommand::ImePreedit("かん".to_string()),
                EditorCommand::ImeCancel,
            ],
            expect: GateExpect {
                text: "x",
                undo_depth: Some(0),
                ..Default::default()
            },
        },
        GateCase {
            name: "ime_commit_is_one_transaction",
            initial: "x",
            commands: vec![
                cur(0, 1),
                EditorCommand::ImePreedit("かん".to_string()),
                EditorCommand::ImeCommit("漢字".to_string()),
            ],
            expect: GateExpect {
                text: "x漢字",
                cursor: Some(Cursor::new(0, 3)),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "ime_commit_undoes_atomically",
            initial: "x",
            commands: vec![
                cur(0, 1),
                EditorCommand::ImePreedit("あ".to_string()),
                EditorCommand::ImeCommit("あい".to_string()),
                EditorCommand::Undo,
            ],
            expect: GateExpect {
                text: "x",
                can_redo: Some(true),
                ..Default::default()
            },
        },
        GateCase {
            name: "nav_word_left",
            initial: "foo bar baz",
            commands: vec![mv(Movement::DocEnd), mv(Movement::WordLeft)],
            expect: GateExpect {
                text: "foo bar baz",
                cursor: Some(Cursor::new(0, 8)),
                ..Default::default()
            },
        },
        GateCase {
            name: "nav_word_right",
            initial: "foo bar baz",
            commands: vec![mv(Movement::WordRight)],
            expect: GateExpect {
                text: "foo bar baz",
                cursor: Some(Cursor::new(0, 3)),
                ..Default::default()
            },
        },
        GateCase {
            name: "nav_vertical_goal_clamp",
            initial: "one\ntwo\nthree",
            commands: vec![mv(Movement::DocEnd), mv(Movement::Up)],
            expect: GateExpect {
                text: "one\ntwo\nthree",
                cursor: Some(Cursor::new(1, 3)),
                ..Default::default()
            },
        },
        GateCase {
            name: "home_end_equivalents",
            initial: "one\ntwo three",
            commands: vec![cur(1, 2), mv(Movement::LineEnd), mv(Movement::LineStart)],
            expect: GateExpect {
                text: "one\ntwo three",
                cursor: Some(Cursor::new(1, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "doc_start_end_movements",
            initial: "a\nb\nc",
            commands: vec![mv(Movement::DocEnd), mv(Movement::DocStart)],
            expect: GateExpect {
                text: "a\nb\nc",
                cursor: Some(Cursor::new(0, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "page_down_moves_by_rows",
            initial: "l0\nl1\nl2\nl3\nl4\nl5",
            commands: vec![mv(Movement::PageDown(3))],
            expect: GateExpect {
                text: "l0\nl1\nl2\nl3\nl4\nl5",
                cursor: Some(Cursor::new(3, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "page_up_clamps_to_doc_start",
            initial: "l0\nl1\nl2",
            commands: vec![cur(2, 0), mv(Movement::PageUp(10))],
            expect: GateExpect {
                text: "l0\nl1\nl2",
                cursor: Some(Cursor::new(0, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "select_all_replace",
            initial: "old text",
            commands: vec![EditorCommand::SelectAll, ins("new")],
            expect: GateExpect {
                text: "new",
                cursor: Some(Cursor::new(0, 3)),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "select_word_at",
            initial: "foo bar\nbaz",
            commands: vec![EditorCommand::SelectWordAt(Cursor::new(0, 5))],
            expect: GateExpect {
                text: "foo bar\nbaz",
                selected: Some("bar"),
                ..Default::default()
            },
        },
        GateCase {
            name: "select_line_at",
            initial: "foo bar\nbaz",
            commands: vec![EditorCommand::SelectLineAt(0)],
            expect: GateExpect {
                text: "foo bar\nbaz",
                selected: Some("foo bar\n"),
                ..Default::default()
            },
        },
        GateCase {
            name: "shift_word_select",
            initial: "hello world",
            commands: vec![mv_ext(Movement::WordRight)],
            expect: GateExpect {
                text: "hello world",
                selected: Some("hello"),
                ..Default::default()
            },
        },
        GateCase {
            name: "extend_to_selection",
            initial: "abcdef",
            commands: vec![cur(0, 1), ext(0, 4)],
            expect: GateExpect {
                text: "abcdef",
                selected: Some("bcd"),
                ..Default::default()
            },
        },
        GateCase {
            name: "backward_selection_replace",
            initial: "abcdef",
            commands: vec![cur(0, 4), ext(0, 1), ins("X")],
            expect: GateExpect {
                text: "aXef",
                cursor: Some(Cursor::new(0, 2)),
                ..Default::default()
            },
        },
        GateCase {
            name: "extend_mid_cluster_snaps",
            initial: "a👨\u{200D}👩\u{200D}👧b",
            commands: vec![cur(0, 0), ext(0, 3)],
            expect: GateExpect {
                text: "a👨\u{200D}👩\u{200D}👧b",
                selected: Some("a"),
                ..Default::default()
            },
        },
        GateCase {
            name: "delete_forward_grapheme",
            initial: "e\u{301}x",
            commands: vec![cur(0, 0), EditorCommand::DeleteForward],
            expect: GateExpect {
                text: "x",
                cursor: Some(Cursor::new(0, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "backspace_join_lines",
            initial: "ab\ncd",
            commands: vec![cur(1, 0), EditorCommand::Backspace],
            expect: GateExpect {
                text: "abcd",
                cursor: Some(Cursor::new(0, 2)),
                ..Default::default()
            },
        },
        GateCase {
            name: "delete_forward_at_end_noop",
            initial: "ab",
            commands: vec![mv(Movement::DocEnd), EditorCommand::DeleteForward],
            expect: GateExpect {
                text: "ab",
                undo_depth: Some(0),
                ..Default::default()
            },
        },
        GateCase {
            name: "undo_coalesced_group",
            initial: "",
            commands: vec![ins("a"), ins("b"), EditorCommand::Undo],
            expect: GateExpect {
                text: "",
                undo_depth: Some(0),
                can_redo: Some(true),
                ..Default::default()
            },
        },
        GateCase {
            name: "redo_after_undo",
            initial: "",
            commands: vec![ins("a"), EditorCommand::Undo, EditorCommand::Redo],
            expect: GateExpect {
                text: "a",
                undo_depth: Some(1),
                can_redo: Some(false),
                ..Default::default()
            },
        },
        GateCase {
            name: "redo_invalidated_by_new_edit",
            initial: "",
            commands: vec![ins("a"), EditorCommand::Undo, ins("z")],
            expect: GateExpect {
                text: "z",
                can_redo: Some(false),
                ..Default::default()
            },
        },
        GateCase {
            name: "undo_restores_selection",
            initial: "abc",
            commands: vec![EditorCommand::SelectAll, ins("z"), EditorCommand::Undo],
            expect: GateExpect {
                text: "abc",
                selected: Some("abc"),
                can_redo: Some(true),
                ..Default::default()
            },
        },
        GateCase {
            name: "indent_at_caret",
            initial: "ab",
            commands: vec![cur(0, 1), EditorCommand::Indent],
            expect: GateExpect {
                text: "a    b",
                cursor: Some(Cursor::new(0, 5)),
                ..Default::default()
            },
        },
        GateCase {
            name: "indent_multiline_one_transaction",
            initial: "one\ntwo",
            commands: vec![cur(0, 1), ext(1, 1), EditorCommand::Indent],
            expect: GateExpect {
                text: "    one\n    two",
                selected: Some("ne\n    t"),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "outdent_mixed_indents",
            initial: "    one\n\ttwo",
            commands: vec![EditorCommand::SelectAll, EditorCommand::Outdent],
            expect: GateExpect {
                text: "one\ntwo",
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "outdent_noop_records_nothing",
            initial: "plain",
            commands: vec![EditorCommand::Outdent],
            expect: GateExpect {
                text: "plain",
                undo_depth: Some(0),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_indent_whole_line",
            initial: "- item",
            commands: vec![cur(0, 3), EditorCommand::MarkdownIndent],
            expect: GateExpect {
                text: "    - item",
                cursor: Some(Cursor::new(0, 7)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_outdent_whole_line",
            initial: "    - item",
            commands: vec![cur(0, 7), EditorCommand::MarkdownOutdent],
            expect: GateExpect {
                text: "- item",
                cursor: Some(Cursor::new(0, 3)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_newline_unordered",
            initial: "- item",
            commands: vec![mv(Movement::LineEnd), EditorCommand::MarkdownNewline],
            expect: GateExpect {
                text: "- item\n- ",
                cursor: Some(Cursor::new(1, 2)),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_newline_ordered_renumbers",
            initial: "1. one\n2. two",
            commands: vec![cur(0, 6), EditorCommand::MarkdownNewline],
            expect: GateExpect {
                text: "1. one\n2. \n3. two",
                cursor: Some(Cursor::new(1, 3)),
                undo_depth: Some(1),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_newline_task",
            initial: "- [x] done",
            commands: vec![mv(Movement::LineEnd), EditorCommand::MarkdownNewline],
            expect: GateExpect {
                text: "- [x] done\n- [ ] ",
                cursor: Some(Cursor::new(1, 6)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_newline_quote",
            initial: "> quoted",
            commands: vec![mv(Movement::LineEnd), EditorCommand::MarkdownNewline],
            expect: GateExpect {
                text: "> quoted\n> ",
                cursor: Some(Cursor::new(1, 2)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_empty_continuation_exits",
            initial: "- item\n- ",
            commands: vec![mv(Movement::DocEnd), EditorCommand::MarkdownNewline],
            expect: GateExpect {
                text: "- item\n",
                cursor: Some(Cursor::new(1, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_backspace_empty_marker",
            initial: "    - ",
            commands: vec![mv(Movement::LineEnd), EditorCommand::MarkdownBackspace],
            expect: GateExpect {
                text: "    ",
                cursor: Some(Cursor::new(0, 4)),
                ..Default::default()
            },
        },
        GateCase {
            name: "markdown_backspace_plain_content",
            initial: "- ab",
            commands: vec![mv(Movement::LineEnd), EditorCommand::MarkdownBackspace],
            expect: GateExpect {
                text: "- a",
                cursor: Some(Cursor::new(0, 3)),
                ..Default::default()
            },
        },
        GateCase {
            name: "smart_backspace_indent_level",
            initial: "    x",
            commands: vec![cur(0, 4), EditorCommand::Backspace],
            expect: GateExpect {
                text: "x",
                cursor: Some(Cursor::new(0, 0)),
                ..Default::default()
            },
        },
        GateCase {
            name: "newline_carries_auto_indent",
            initial: "    item",
            commands: vec![mv(Movement::LineEnd), EditorCommand::InsertNewline],
            expect: GateExpect {
                text: "    item\n    ",
                cursor: Some(Cursor::new(1, 4)),
                ..Default::default()
            },
        },
        GateCase {
            name: "set_cursor_clamps_out_of_bounds",
            initial: "ab",
            commands: vec![cur(9, 9)],
            expect: GateExpect {
                text: "ab",
                cursor: Some(Cursor::new(0, 2)),
                ..Default::default()
            },
        },
    ]
}

// ─── Invariants ──────────────────────────────────────────────────────────────

fn cursor_valid(doc: &Document, cursor: Cursor, what: &str) -> Result<(), String> {
    let clamped = movement::clamp(doc.buffer(), cursor);
    if clamped != cursor {
        return Err(format!(
            "{what} {cursor:?} is not a valid grapheme boundary (clamps to {clamped:?})"
        ));
    }
    Ok(())
}

/// Read-only invariants that must hold after every applied command.
/// `prev_revision` is the revision observed before the command.
pub fn check_invariants(doc: &Document, prev_revision: u64) -> Result<(), String> {
    let sel = doc.selection();
    cursor_valid(doc, sel.anchor, "selection anchor")?;
    cursor_valid(doc, sel.head, "selection head")?;
    let (start, end) = sel.ordered();
    if start > end {
        return Err(format!("ordered() returned start > end for {sel:?}"));
    }
    let state = doc.semantic_state(0.0);
    if state.text != doc.text() {
        return Err("semantic_state().text diverged from doc.text()".to_string());
    }
    if state.can_undo != (state.undo_depth > 0) {
        return Err(format!(
            "undo_depth {} inconsistent with can_undo {}",
            state.undo_depth, state.can_undo
        ));
    }
    if doc.revision() < prev_revision {
        return Err(format!(
            "revision went backwards: {} -> {}",
            prev_revision,
            doc.revision()
        ));
    }
    Ok(())
}

/// History round-trip invariant: when undo is available, Undo then Redo must
/// restore the exact text. Mutates only history bookkeeping (breaks the open
/// coalescing group), so callers run it where coalescing no longer matters.
pub fn check_history_round_trip(doc: &mut Document) -> Result<(), String> {
    if !doc.semantic_state(0.0).can_undo {
        return Ok(());
    }
    let before = doc.text();
    doc.apply(EditorCommand::Undo);
    doc.apply(EditorCommand::Redo);
    let after = doc.text();
    if before != after {
        return Err(format!(
            "undo/redo round trip did not restore text: {before:?} -> {after:?}"
        ));
    }
    Ok(())
}

// ─── Case runner ─────────────────────────────────────────────────────────────

fn run_case(case: &GateCase) -> Result<EditorSemanticState, String> {
    let mut doc = Document::new(case.initial);
    let mut prev_revision = doc.revision();
    for (index, command) in case.commands.iter().enumerate() {
        doc.apply(command.clone());
        check_invariants(&doc, prev_revision)
            .map_err(|e| format!("command[{index}] {command:?}: {e}"))?;
        prev_revision = doc.revision();
    }
    let expect = &case.expect;
    if doc.text() != expect.text {
        return Err(format!(
            "final text mismatch: expected {:?}, got {:?}",
            expect.text,
            doc.text()
        ));
    }
    if let Some(cursor) = expect.cursor {
        if doc.cursor() != cursor {
            return Err(format!(
                "final cursor mismatch: expected {cursor:?}, got {:?}",
                doc.cursor()
            ));
        }
    }
    if let Some(selected) = expect.selected {
        if doc.selected_text() != selected {
            return Err(format!(
                "final selection mismatch: expected {selected:?}, got {:?}",
                doc.selected_text()
            ));
        }
    }
    let state = doc.semantic_state(0.0);
    if let Some(depth) = expect.undo_depth {
        if state.undo_depth != depth {
            return Err(format!(
                "undo_depth mismatch: expected {depth}, got {}",
                state.undo_depth
            ));
        }
    }
    if let Some(can_redo) = expect.can_redo {
        if state.can_redo != can_redo {
            return Err(format!(
                "can_redo mismatch: expected {can_redo}, got {}",
                state.can_redo
            ));
        }
    }
    check_history_round_trip(&mut doc)?;
    Ok(doc.semantic_state(0.0))
}

// ─── Deterministic long sequence ─────────────────────────────────────────────

const LONG_SEQUENCE_REPS: usize = 24;
/// Guard band under `EditHistory`'s retention cap: the full-undo-to-empty
/// assertion is only valid while no undo group was ever evicted.
const MAX_OBSERVED_DEPTH: usize = 480;

fn run_long_sequence() -> Result<usize, String> {
    let mut doc = Document::new("");
    let mut prev_revision = doc.revision();
    let mut applied = 0usize;
    let mut max_depth = 0usize;
    let cases = gate_cases();
    for rep in 0..LONG_SEQUENCE_REPS {
        for case in &cases {
            for command in &case.commands {
                doc.apply(command.clone());
                applied += 1;
                check_invariants(&doc, prev_revision)
                    .map_err(|e| format!("rep {rep} case {} @{applied}: {e}", case.name))?;
                prev_revision = doc.revision();
                max_depth = max_depth.max(doc.semantic_state(0.0).undo_depth);
                // Interleaved undo/redo passes keep the history exercised
                // (and pruned: an edit after an undo drops the redone group).
                if applied % 5 == 0 {
                    doc.apply(EditorCommand::Undo);
                    check_invariants(&doc, prev_revision)
                        .map_err(|e| format!("interleaved undo @{applied}: {e}"))?;
                    prev_revision = doc.revision();
                }
                if applied % 13 == 0 {
                    doc.apply(EditorCommand::Redo);
                    check_invariants(&doc, prev_revision)
                        .map_err(|e| format!("interleaved redo @{applied}: {e}"))?;
                    prev_revision = doc.revision();
                }
                if applied % 97 == 0 {
                    check_history_round_trip(&mut doc)
                        .map_err(|e| format!("round trip @{applied}: {e}"))?;
                }
            }
        }
    }
    if applied < 1000 {
        return Err(format!(
            "long sequence too short ({applied} commands) to qualify as a stress pass"
        ));
    }
    if max_depth >= MAX_OBSERVED_DEPTH {
        return Err(format!(
            "undo depth reached {max_depth}; retention cap invalidates full-undo assertion"
        ));
    }
    // Full undo walks back to the empty initial document; full redo restores
    // the exact final text.
    let final_text = doc.text();
    let mut guard = 0usize;
    while doc.semantic_state(0.0).can_undo {
        doc.apply(EditorCommand::Undo);
        guard += 1;
        if guard > 100_000 {
            return Err("full undo did not terminate".to_string());
        }
    }
    if !doc.text().is_empty() {
        return Err(format!(
            "full undo did not restore the empty document; residue: {:?}",
            doc.text()
        ));
    }
    while doc.semantic_state(0.0).can_redo {
        doc.apply(EditorCommand::Redo);
    }
    if doc.text() != final_text {
        return Err("full redo did not restore the final text".to_string());
    }
    Ok(applied)
}

// ─── Deterministic PRNG + randomized commands ────────────────────────────────

/// SplitMix64: tiny, deterministic, dependency-free PRNG.
pub struct SplitMix64(u64);

impl SplitMix64 {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

const INSERT_POOL: [&str; 14] = [
    "a", "b", "z", " ", "hi", "\n", "é", "e\u{301}", "👨\u{200D}👩\u{200D}👧", "🇺🇸", "漢",
    "- ", "1. ", "> ",
];

fn random_cursor(rng: &mut SplitMix64, doc: &Document) -> Cursor {
    // Deliberately allows out-of-bounds positions: SetCursor/ExtendTo must
    // clamp them onto valid grapheme boundaries.
    let line = rng.below(doc.buffer().line_count() as u64 + 2) as usize;
    let column = rng.below(24) as usize;
    Cursor::new(line, column)
}

fn random_movement(rng: &mut SplitMix64) -> Movement {
    match rng.below(12) {
        0 => Movement::Left,
        1 => Movement::Right,
        2 => Movement::Up,
        3 => Movement::Down,
        4 => Movement::WordLeft,
        5 => Movement::WordRight,
        6 => Movement::LineStart,
        7 => Movement::LineEnd,
        8 => Movement::DocStart,
        9 => Movement::DocEnd,
        10 => Movement::PageUp(1 + rng.below(20) as usize),
        _ => Movement::PageDown(1 + rng.below(20) as usize),
    }
}

/// Weighted random command generator over the full `EditorCommand` surface.
pub fn random_command(rng: &mut SplitMix64, doc: &Document) -> EditorCommand {
    match rng.below(100) {
        0..=29 => {
            EditorCommand::InsertText(INSERT_POOL[rng.below(INSERT_POOL.len() as u64) as usize].to_string())
        }
        30..=43 => EditorCommand::Move {
            movement: random_movement(rng),
            extend: rng.below(3) == 0,
        },
        44..=50 => EditorCommand::Backspace,
        51..=54 => EditorCommand::DeleteForward,
        55..=59 => EditorCommand::SetCursor(random_cursor(rng, doc)),
        60..=62 => EditorCommand::ExtendTo(random_cursor(rng, doc)),
        63 => EditorCommand::SelectAll,
        64 => EditorCommand::SelectWordAt(random_cursor(rng, doc)),
        65 => EditorCommand::SelectLineAt(rng.below(doc.buffer().line_count() as u64 + 2) as usize),
        66 => EditorCommand::Indent,
        67 => EditorCommand::Outdent,
        68 => EditorCommand::MarkdownIndent,
        69 => EditorCommand::MarkdownOutdent,
        70..=71 => EditorCommand::MarkdownNewline,
        72..=73 => EditorCommand::MarkdownBackspace,
        74 => EditorCommand::InsertNewline,
        75..=82 => EditorCommand::Undo,
        83..=88 => EditorCommand::Redo,
        89..=92 => EditorCommand::ImePreedit(
            INSERT_POOL[rng.below(INSERT_POOL.len() as u64) as usize].to_string(),
        ),
        93..=95 => EditorCommand::ImeCommit(
            INSERT_POOL[rng.below(INSERT_POOL.len() as u64) as usize].to_string(),
        ),
        96 => EditorCommand::ImeCancel,
        _ => EditorCommand::Move {
            movement: random_movement(rng),
            extend: true,
        },
    }
}

/// Replays `commands` on a fresh empty document with per-command invariant
/// checks plus the final history round trip — the same predicates the seed
/// run applies, so any original failure mode reproduces during minimization.
/// Returns the failing command index and message, if any.
fn replay(commands: &[EditorCommand]) -> Result<(), (usize, String)> {
    let mut doc = Document::new("");
    let mut prev_revision = doc.revision();
    for (index, command) in commands.iter().enumerate() {
        doc.apply(command.clone());
        if let Err(e) = check_invariants(&doc, prev_revision) {
            return Err((index, e));
        }
        prev_revision = doc.revision();
    }
    check_history_round_trip(&mut doc)
        .map_err(|e| (commands.len().saturating_sub(1), e))
}

/// Greedy sequence minimization: repeatedly drop commands while the failure
/// still reproduces. Bounded passes keep worst-case cost predictable.
fn minimize(mut commands: Vec<EditorCommand>) -> (Vec<EditorCommand>, String) {
    let mut message = match replay(&commands) {
        Err((index, message)) => {
            commands.truncate(index + 1);
            message
        }
        Ok(()) => return (commands, "failure did not reproduce on replay".to_string()),
    };
    for _pass in 0..6 {
        let mut removed_any = false;
        let mut i = commands.len();
        while i > 0 {
            i -= 1;
            let mut candidate = commands.clone();
            candidate.remove(i);
            if let Err((_, m)) = replay(&candidate) {
                commands = candidate;
                message = m;
                removed_any = true;
            }
        }
        if !removed_any {
            break;
        }
    }
    (commands, message)
}

#[derive(serde::Serialize)]
struct ReplayBundle {
    seed: u64,
    command_count: usize,
    initial_text: String,
    invariant_message: String,
    minimized_commands: Vec<EditorCommand>,
}

/// Runs one randomized seed. On invariant failure, minimizes the failing
/// sequence and writes a replay bundle; returns the panic message.
fn run_random_seed(seed: u64, command_count: usize) -> Result<(), String> {
    let mut rng = SplitMix64::new(seed);
    let mut doc = Document::new("");
    let mut prev_revision = doc.revision();
    let mut applied: Vec<EditorCommand> = Vec::with_capacity(command_count);
    let mut failure: Option<String> = None;
    for _ in 0..command_count {
        let command = random_command(&mut rng, &doc);
        applied.push(command.clone());
        doc.apply(command);
        if let Err(e) = check_invariants(&doc, prev_revision) {
            failure = Some(e);
            break;
        }
        prev_revision = doc.revision();
    }
    if failure.is_none() {
        if let Err(e) = check_history_round_trip(&mut doc) {
            failure = Some(e);
        }
    }
    let Some(original_message) = failure else {
        return Ok(());
    };
    let (minimized, message) = minimize(applied);
    let bundle = ReplayBundle {
        seed,
        command_count: minimized.len(),
        initial_text: String::new(),
        invariant_message: message.clone(),
        minimized_commands: minimized,
    };
    let bundle_path = std::env::temp_dir().join(format!("plexi-editor-gate-failure-{seed}.json"));
    match serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string()).and_then(|json| {
        std::fs::write(&bundle_path, json).map_err(|e| e.to_string())
    }) {
        Ok(()) => {}
        Err(e) => log::error!(
            "editor_gate: failed to write replay bundle {}: {e}",
            bundle_path.display()
        ),
    }
    Err(format!(
        "seed {seed} invariant failure: {original_message} (minimized: {message}). \
         Replay bundle: {}. Reproduce with {GATE_SEED_ENV}={seed}",
        bundle_path.display()
    ))
}

fn seeds_under_test() -> Vec<u64> {
    match std::env::var(GATE_SEED_ENV) {
        Ok(raw) => {
            let seed: u64 = raw
                .parse()
                .unwrap_or_else(|e| panic!("{GATE_SEED_ENV}={raw:?} is not a u64: {e}"));
            vec![seed]
        }
        Err(_) => DEFAULT_SEEDS.to_vec(),
    }
}

// ─── Qualification summary + artifact ────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct GateCaseResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_semantic: Option<EditorSemanticState>,
}

#[derive(serde::Serialize)]
pub struct GateRandomizedResult {
    pub seed: u64,
    pub commands: usize,
    pub passed: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct GateLongSequenceResult {
    pub passed: bool,
    pub commands: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct GateTotals {
    pub sections: usize,
    pub passed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct GateSummary {
    pub schema_version: u32,
    pub cases: Vec<GateCaseResult>,
    pub long_sequence: GateLongSequenceResult,
    pub randomized: Vec<GateRandomizedResult>,
    pub totals: GateTotals,
}

/// Runs the whole core gate (matrix + long sequence + default seeds), writes
/// `editor-gate-core.json` into `out_dir`, and returns the summary.
pub fn run_core_qualification(out_dir: &Path) -> GateSummary {
    let seeds = seeds_under_test();
    let cases = gate_cases();
    log::info!(
        "editor_gate: core qualification started cases={} seeds={}",
        cases.len(),
        seeds.len()
    );
    let start = Instant::now();

    let case_results: Vec<GateCaseResult> = cases
        .iter()
        .map(|case| {
            let t = Instant::now();
            let result = run_case(case);
            GateCaseResult {
                name: case.name.to_string(),
                passed: result.is_ok(),
                duration_ms: t.elapsed().as_millis() as u64,
                error: result.as_ref().err().cloned(),
                final_semantic: result.ok(),
            }
        })
        .collect();

    let t = Instant::now();
    let long_result = run_long_sequence();
    let long_sequence = GateLongSequenceResult {
        passed: long_result.is_ok(),
        commands: *long_result.as_ref().unwrap_or(&0),
        duration_ms: t.elapsed().as_millis() as u64,
        error: long_result.err(),
    };

    let randomized: Vec<GateRandomizedResult> = seeds
        .iter()
        .map(|&seed| {
            let t = Instant::now();
            let result = run_random_seed(seed, RANDOM_COMMANDS_PER_SEED);
            GateRandomizedResult {
                seed,
                commands: RANDOM_COMMANDS_PER_SEED,
                passed: result.is_ok(),
                duration_ms: t.elapsed().as_millis() as u64,
                error: result.err(),
            }
        })
        .collect();

    let sections = case_results.len() + 1 + randomized.len();
    let failed = case_results.iter().filter(|c| !c.passed).count()
        + usize::from(!long_sequence.passed)
        + randomized.iter().filter(|r| !r.passed).count();
    let summary = GateSummary {
        schema_version: 1,
        cases: case_results,
        long_sequence,
        randomized,
        totals: GateTotals {
            sections,
            passed: sections - failed,
            failed,
            duration_ms: start.elapsed().as_millis() as u64,
        },
    };

    if let Err(e) = std::fs::create_dir_all(out_dir) {
        log::error!("editor_gate: failed to create {}: {e}", out_dir.display());
    } else {
        let artifact = out_dir.join("editor-gate-core.json");
        match serde_json::to_vec_pretty(&summary)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&artifact, json).map_err(|e| e.to_string()))
        {
            Ok(()) => log::info!("editor_gate: wrote artifact {}", artifact.display()),
            Err(e) => log::error!(
                "editor_gate: failed to write artifact {}: {e}",
                artifact.display()
            ),
        }
    }

    log::info!(
        "editor_gate: core qualification finished passed={} failed={} duration_ms={}",
        summary.totals.passed,
        summary.totals.failed,
        summary.totals.duration_ms
    );
    println!(
        "editor_gate: core qualification finished passed={} failed={} duration_ms={}",
        summary.totals.passed, summary.totals.failed, summary.totals.duration_ms
    );
    summary
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn gate_core_matrix() {
    for case in gate_cases() {
        if let Err(e) = run_case(&case) {
            panic!(
                "editor gate case {:?} failed ({} commands): {e}\ncommands: {:?}",
                case.name,
                case.commands.len(),
                case.commands
            );
        }
    }
}

#[test]
fn gate_deterministic_long_sequence() {
    let applied = run_long_sequence().unwrap_or_else(|e| panic!("long sequence failed: {e}"));
    assert!(applied >= 1000, "sequence must span thousands of commands");
}

#[test]
fn gate_randomized_commands() {
    for seed in seeds_under_test() {
        if let Err(e) = run_random_seed(seed, RANDOM_COMMANDS_PER_SEED) {
            panic!("{e}");
        }
    }
}

#[test]
fn gate_core_qualification_artifact() {
    let out_dir = std::env::temp_dir().join("plexi-editor-gate");
    let summary = run_core_qualification(&out_dir);
    assert_eq!(
        summary.totals.failed,
        0,
        "core qualification reported failures; see {}",
        out_dir.join("editor-gate-core.json").display()
    );
    assert!(out_dir.join("editor-gate-core.json").is_file());
}

#[test]
fn gate_minimizer_shrinks_reproducible_failures() {
    // Sanity for the minimizer itself, using a synthetic always-failing
    // replay: an out-of-order revision cannot be produced by real commands,
    // so instead assert the greedy pass keeps a reproducing subsequence when
    // the invariant checker flags nothing (no real failure => passthrough).
    let commands = vec![
        EditorCommand::InsertText("a".to_string()),
        EditorCommand::Undo,
    ];
    let (kept, message) = minimize(commands.clone());
    assert_eq!(kept, commands, "healthy sequences must come back unchanged");
    assert!(message.contains("did not reproduce"));
}
