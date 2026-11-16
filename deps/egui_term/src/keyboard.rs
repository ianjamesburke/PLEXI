//! Kitty encoding for the key information egui exposes. Protocol negotiation
//! and the independent main/alternate-screen stacks belong to alacritty.
use alacritty_terminal::term::TermMode;
use egui::{Event, Key, Modifiers};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Code {
    number: u32,
    suffix: char,
    text: bool,
}

/// One physical key's hold, from its non-repeat press to its release. Its
/// repeats and release follow the press, whatever the modifiers or kitty
/// flags do mid-hold.
#[derive(Clone, Copy)]
enum Hold {
    /// The press scrolled the scrollback: repeats scroll while they still
    /// match, and nothing of the hold reaches the program.
    Scroll,
    /// The press reached the program.
    Program(Press),
    /// `release_held` already reported this hold released: its repeats and
    /// physical release never reach the program.
    Released,
}

/// The press that started a program hold, kept so `release_held` can report
/// its release with the press's code and modifiers.
#[derive(Clone, Copy)]
struct Press {
    key: Key,
    physical: Option<Key>,
    mods: Modifiers,
    /// The kitty code sent, when the press was kitty-encoded.
    code: Option<Code>,
}

#[derive(Clone, Default)]
pub(crate) struct KeyboardEncoder {
    /// Per physical key; a non-repeat press starts a new hold, so an entry
    /// left by a lost release never outlives the next press.
    holds: HashMap<Key, Hold>,
}

pub(crate) struct PreparedEvent {
    pub event: Event,
    /// None delegates to the existing legacy binding path; an empty encoding
    /// consumes the event without writing (e.g. release reporting is disabled).
    pub encoding: Option<Vec<u8>>,
}

impl KeyboardEncoder {
    /// `scrolls` reports whether a press scrolls ahead of kitty encoding
    /// (`BindingsLayout::scrolls_before_encoding`). Such a press, and the
    /// repeats of its hold that still match, fall through to the binding
    /// path; the rest of that hold never reaches the program.
    pub fn prepare(
        &mut self,
        events: Vec<Event>,
        mode: TermMode,
        scrolls: impl Fn(Key, Modifiers) -> bool,
    ) -> Vec<PreparedEvent> {
        let mode = mode & TermMode::KITTY_KEYBOARD_PROTOCOL;
        let mut events = events.into_iter().peekable();
        let mut result = Vec::new();
        while let Some(event) = events.next() {
            let encoding = match &event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => 'key: {
                    let identity = physical_key.unwrap_or(*key);
                    if *pressed && !*repeat {
                        self.holds.remove(&identity);
                    }
                    let press = Press {
                        key: *key,
                        physical: *physical_key,
                        mods: *modifiers,
                        code: None,
                    };
                    // A press (or an ownerless repeat) that starts a hold
                    // records its kitty code below.
                    let mut starts_hold = false;
                    let hold = if *pressed {
                        *self.holds.entry(identity).or_insert_with(|| {
                            starts_hold = true;
                            if !scrolls(*key, *modifiers) {
                                return Hold::Program(press);
                            }
                            if !mode.is_empty() {
                                log::info!(
                                    "kitty keyboard: {key:?} {modifiers:?} scrolls the scrollback instead of reaching the program"
                                );
                            }
                            Hold::Scroll
                        })
                    } else {
                        match self.holds.remove(&identity) {
                            Some(hold) => hold,
                            // This terminal never saw the press (it went to
                            // another pane): the program must not see a
                            // release.
                            None if !mode.is_empty() => {
                                break 'key Some(Vec::new())
                            },
                            None => Hold::Program(press),
                        }
                    };
                    let held = match hold {
                        Hold::Scroll => {
                            let still_scrolls =
                                *pressed && scrolls(*key, *modifiers);
                            break 'key (!still_scrolls).then(Vec::new);
                        },
                        Hold::Released => break 'key Some(Vec::new()),
                        Hold::Program(held) => held,
                    };
                    // A program-owned hold never scrolls, whatever the
                    // modifiers or flags did since its press.
                    let to_program = *pressed && scrolls(*key, *modifiers);
                    if mode.is_empty() {
                        break 'key to_program.then(|| {
                            xterm_modified(*key, *physical_key, *modifiers)
                        });
                    }
                    // egui-winit emits Key followed by Text for a printable
                    // press. Consume that text only when this key was encoded.
                    let text = if *pressed {
                        match events.peek() {
                            Some(Event::Text(text)) => Some(text.as_str()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let code = if !pressed {
                        held.code.or_else(|| key_code(*key, *modifiers))
                    } else {
                        let mut code = key_code(*key, *modifiers);
                        // For layouts not represented by egui::Key, egui uses
                        // the physical key. Preserve the actual Unicode text.
                        if let Some(text) = text
                            .filter(|text| !text.is_ascii() && !modifiers.alt)
                        {
                            let mut chars = text.chars();
                            if let (Some(c), None) =
                                (chars.next(), chars.next())
                            {
                                let mut lower = c.to_lowercase();
                                if let (Some(c), None) =
                                    (lower.next(), lower.next())
                                {
                                    code = Some(Code {
                                        number: c as u32,
                                        suffix: 'u',
                                        text: true,
                                    });
                                }
                            }
                        }
                        if starts_hold {
                            self.holds.insert(
                                identity,
                                Hold::Program(Press { code, ..press }),
                            );
                        }
                        code
                    };
                    let encoded = code.and_then(|code| {
                        encode_key(
                            code,
                            *physical_key,
                            *modifiers,
                            *pressed,
                            *repeat,
                            text,
                            mode,
                        )
                    });
                    if encoded.is_some() && text.is_some() {
                        events.next();
                    }
                    if encoded.is_none() && to_program {
                        break 'key Some(xterm_modified(
                            *key,
                            *physical_key,
                            *modifiers,
                        ));
                    }
                    encoded
                },
                // IME/text-only input has no physical key. Never discard it
                // when an application requests escape-coded text.
                Event::Text(text)
                    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) =>
                {
                    Some(encode_text(text, mode).into_bytes())
                },
                _ => None,
            };
            result.push(PreparedEvent { event, encoding });
        }
        result
    }

    /// Whether a program hold is open, so callers can skip `release_held`.
    pub fn is_holding(&self) -> bool {
        self.holds
            .values()
            .any(|hold| matches!(hold, Hold::Program(_)))
    }

    /// Ends every program hold, for when this terminal stops receiving the
    /// keyboard (window focus loss, focus moving to another pane): the
    /// physical releases will never arrive here. Returns the release reports
    /// the program asked for under the current `mode`, each with its press's
    /// code and modifiers. A reported hold's later repeats and physical
    /// release never reach the program; a hold with nothing to report is
    /// forgotten. Scroll holds keep their ownership until their release.
    pub fn release_held(&mut self, mode: TermMode) -> Vec<u8> {
        let mode = mode & TermMode::KITTY_KEYBOARD_PROTOCOL;
        let mut bytes = Vec::new();
        self.holds.retain(|_, hold| {
            let Hold::Program(press) = *hold else {
                return true;
            };
            let release = press
                .code
                .or_else(|| key_code(press.key, press.mods))
                .and_then(|code| {
                    encode_key(
                        code,
                        press.physical,
                        press.mods,
                        false,
                        false,
                        None,
                        mode,
                    )
                })
                .unwrap_or_default();
            if release.is_empty() {
                return false;
            }
            bytes.extend(release);
            *hold = Hold::Released;
            true
        });
        bytes
    }
}

/// The xterm modified-key form (`CSI 5;2~`, `CSI 1;9H`), which kitty's
/// disambiguation keeps for these keys: how a program-owned hold reaches the
/// program when a scroll binding would otherwise take it.
fn xterm_modified(key: Key, physical: Option<Key>, mods: Modifiers) -> Vec<u8> {
    key_code(key, mods)
        .and_then(|code| {
            encode_key(
                code,
                physical,
                mods,
                true,
                false,
                None,
                TermMode::DISAMBIGUATE_ESC_CODES,
            )
        })
        .unwrap_or_default()
}

fn encode_key(
    code: Code,
    physical: Option<Key>,
    mods: Modifiers,
    pressed: bool,
    repeat: bool,
    text: Option<&str>,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let types = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES);
    if !pressed && !types {
        return Some(Vec::new());
    }
    // Kitty deliberately preserves these C0 keys unless report-all is set.
    if !all && code.suffix == 'u' && matches!(code.number, 9 | 13 | 127) {
        return if pressed { None } else { Some(Vec::new()) };
    }
    let chord = mods.ctrl || mods.alt || mods.mac_cmd;
    let encode = all
        || (code.text && (disambiguate || types) && chord)
        || (!code.text && (disambiguate || types));
    if !encode {
        return if pressed { None } else { Some(Vec::new()) };
    }
    let mut key = code.number.to_string();
    if code.text && mode.contains(TermMode::REPORT_ALTERNATE_KEYS) {
        let shifted = text.filter(|_| mods.shift).and_then(|s| {
            let mut chars = s.chars();
            let c = chars.next()?;
            (chars.next().is_none() && c as u32 != code.number)
                .then_some(c as u32)
        });
        let base = physical
            .and_then(|k| key_code(k, Modifiers::NONE))
            .filter(|c| c.text && c.number != code.number)
            .map(|c| c.number);
        if shifted.is_some() || base.is_some() {
            key.push(':');
            if let Some(c) = shifted {
                key.push_str(&c.to_string());
            }
            if let Some(c) = base {
                key.push(':');
                key.push_str(&c.to_string());
            }
        }
    }
    let modifiers = 1
        + u8::from(mods.shift)
        + 2 * u8::from(mods.alt)
        + 4 * u8::from(mods.ctrl)
        + 8 * u8::from(mods.mac_cmd);
    let event_type = if !pressed {
        3
    } else if repeat {
        2
    } else {
        1
    };
    let mut sequence = format!("\x1b[{key};{modifiers}");
    if types {
        sequence.push_str(&format!(":{event_type}"));
    }
    if pressed && mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) {
        if let Some(text) = text {
            append_text(&mut sequence, text);
        }
    }
    sequence.push(code.suffix);
    Some(sequence.into_bytes())
}

fn append_text(sequence: &mut String, text: &str) {
    let mut separator = ';';
    for c in text.chars().filter(|c| !c.is_control()) {
        sequence.push(separator);
        sequence.push_str(&(c as u32).to_string());
        separator = ':';
    }
}

fn encode_text(text: &str, mode: TermMode) -> String {
    if mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) {
        let mut sequence = String::from("\x1b[0;1");
        append_text(&mut sequence, text);
        sequence.push('u');
        sequence
    } else {
        text.chars()
            .filter(|c| !c.is_control())
            .map(|c| format!("\x1b[{};1u", c as u32))
            .collect()
    }
}

fn key_code(key: Key, mods: Modifiers) -> Option<Code> {
    let (number, suffix) = match key {
        Key::Escape => (27, 'u'),
        Key::Enter => (13, 'u'),
        Key::Tab => (9, 'u'),
        Key::Backspace => (127, 'u'),
        Key::Insert => (2, '~'),
        Key::Delete => (3, '~'),
        Key::PageUp => (5, '~'),
        Key::PageDown => (6, '~'),
        Key::ArrowUp => (1, 'A'),
        Key::ArrowDown => (1, 'B'),
        Key::ArrowRight => (1, 'C'),
        Key::ArrowLeft => (1, 'D'),
        Key::Home => (1, 'H'),
        Key::End => (1, 'F'),
        Key::F1 => (1, 'P'),
        Key::F2 => (1, 'Q'),
        Key::F3 => (13, '~'),
        Key::F4 => (1, 'S'),
        Key::F5 => (15, '~'),
        Key::F6 => (17, '~'),
        Key::F7 => (18, '~'),
        Key::F8 => (19, '~'),
        Key::F9 => (20, '~'),
        Key::F10 => (21, '~'),
        Key::F11 => (23, '~'),
        Key::F12 => (24, '~'),
        _ => {
            if let Some(n) = key
                .name()
                .strip_prefix('F')
                .and_then(|n| n.parse::<u32>().ok())
            {
                if (13..=35).contains(&n) {
                    return Some(Code {
                        number: 57376 + n - 13,
                        suffix: 'u',
                        text: false,
                    });
                }
            }
            let c = match key {
                Key::Space => ' ',
                Key::Minus => '-',
                Key::Quote => '\'',
                Key::Colon if mods.shift => ';',
                Key::Plus if mods.shift => '=',
                Key::Pipe if mods.shift => '\\',
                Key::Questionmark if mods.shift => '/',
                Key::Exclamationmark if mods.shift => '1',
                Key::OpenCurlyBracket if mods.shift => '[',
                Key::CloseCurlyBracket if mods.shift => ']',
                _ => {
                    let mut chars = key.symbol_or_name().chars();
                    let c = chars.next()?;
                    if chars.next().is_some() {
                        return None;
                    }
                    c.to_ascii_lowercase()
                },
            };
            return Some(Code {
                number: c as u32,
                suffix: 'u',
                text: true,
            });
        },
    };
    Some(Code {
        number,
        suffix,
        text: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, mods: Modifiers, pressed: bool, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            modifiers: mods,
            pressed,
            repeat,
        }
    }

    fn unbound(_: Key, _: Modifiers) -> bool {
        false
    }

    fn encoded(events: Vec<Event>, mode: TermMode) -> Vec<Option<String>> {
        KeyboardEncoder::default()
            .prepare(events, mode, unbound)
            .into_iter()
            .map(|e| e.encoding.map(|v| String::from_utf8(v).unwrap()))
            .collect()
    }

    #[test]
    fn kitty_plain_text_requires_report_all_for_event_types() {
        let events = vec![
            key(Key::J, Modifiers::NONE, true, false),
            Event::Text("j".into()),
            key(Key::J, Modifiers::NONE, true, true),
            Event::Text("j".into()),
            key(Key::J, Modifiers::NONE, false, false),
        ];
        assert_eq!(
            encoded(events.clone(), TermMode::REPORT_EVENT_TYPES),
            [None, None, None, None, Some(String::new())]
        );
        assert_eq!(
            encoded(
                events,
                TermMode::REPORT_EVENT_TYPES | TermMode::REPORT_ALL_KEYS_AS_ESC
            ),
            [
                Some("\x1b[106;1:1u".into()),
                Some("\x1b[106;1:2u".into()),
                Some("\x1b[106;1:3u".into())
            ]
        );
    }

    #[test]
    fn kitty_disambiguation_preserves_text_and_c0_compatibility() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES;
        assert_eq!(
            encoded(vec![key(Key::Escape, Modifiers::NONE, true, false)], mode),
            [Some("\x1b[27;1u".into())]
        );
        assert_eq!(
            encoded(
                vec![key(
                    Key::I,
                    Modifiers::CTRL | Modifiers::SHIFT,
                    true,
                    false
                )],
                mode
            ),
            [Some("\x1b[105;6u".into())]
        );
        for k in [Key::Enter, Key::Tab, Key::Backspace] {
            assert_eq!(
                encoded(
                    vec![
                        key(k, Modifiers::NONE, true, false),
                        key(k, Modifiers::NONE, false, false)
                    ],
                    mode | TermMode::REPORT_EVENT_TYPES
                ),
                [None, Some(String::new())]
            );
        }
        assert_eq!(
            encoded(
                vec![
                    key(Key::A, Modifiers::SHIFT, true, false),
                    Event::Text("A".into())
                ],
                mode
            ),
            [None, None]
        );
    }

    #[test]
    fn kitty_function_keys_keep_their_protocol_suffix() {
        let mode =
            TermMode::REPORT_EVENT_TYPES | TermMode::REPORT_ALL_KEYS_AS_ESC;
        for (k, expected) in [
            (Key::ArrowUp, "\x1b[1;1:3A"),
            (Key::Delete, "\x1b[3;1:3~"),
            (Key::F3, "\x1b[13;1:3~"),
            (Key::F13, "\x1b[57376;1:3u"),
            (Key::F35, "\x1b[57398;1:3u"),
            (Key::Enter, "\x1b[13;1:3u"),
        ] {
            assert_eq!(
                encoded(
                    vec![
                        key(k, Modifiers::NONE, true, false),
                        key(k, Modifiers::NONE, false, false)
                    ],
                    mode
                )[1],
                Some(expected.into())
            );
        }
        assert_eq!(
            encoded(
                vec![
                    key(Key::F3, Modifiers::NONE, true, false),
                    key(Key::F3, Modifiers::NONE, false, false)
                ],
                TermMode::REPORT_EVENT_TYPES
            )[1],
            Some("\x1b[13;1:3~".into())
        );
        assert_eq!(
            encoded(
                vec![key(Key::J, Modifiers::CTRL, true, true)],
                TermMode::REPORT_EVENT_TYPES
            ),
            [Some("\x1b[106;5:2u".into())]
        );
    }

    #[test]
    fn kitty_pairs_text_once_and_preserves_unicode_and_ime() {
        let mode = TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ASSOCIATED_TEXT;
        let mut encoder = KeyboardEncoder::default();
        let result = encoder.prepare(
            vec![
                key(Key::A, Modifiers::SHIFT, true, false),
                Event::Text("Ж".into()),
            ],
            mode,
            unbound,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].encoding.as_deref(),
            Some(b"\x1b[1078;2:1;1046u".as_slice())
        );
        // The release may have no text and a different modifier state.
        let result = encoder.prepare(
            vec![key(Key::A, Modifiers::NONE, false, false)],
            mode,
            unbound,
        );
        assert_eq!(
            result[0].encoding.as_deref(),
            Some(b"\x1b[1078;1:3u".as_slice())
        );
        assert_eq!(
            encoded(vec![Event::Text("日本".into())], mode),
            [Some("\x1b[0;1;26085:26412u".into())]
        );
        let result = encoder.prepare(
            vec![Event::Paste("paste me".into())],
            mode,
            unbound,
        );
        assert!(
            matches!(&result[0].event, Event::Paste(text) if text == "paste me")
        );
        assert!(result[0].encoding.is_none());
    }

    #[test]
    fn kitty_shifted_punctuation_and_alternate_keys() {
        let mode =
            TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ALTERNATE_KEYS;
        assert_eq!(
            encoded(
                vec![
                    key(Key::Plus, Modifiers::SHIFT, true, false),
                    Event::Text("+".into())
                ],
                mode
            ),
            [Some("\x1b[61:43:43;2u".into())]
        );
        assert_eq!(
            encoded(
                vec![
                    key(Key::A, Modifiers::SHIFT, true, false),
                    Event::Text("A".into())
                ],
                mode
            ),
            [Some("\x1b[97:65;2u".into())]
        );
        assert_eq!(
            encoded(vec![key(Key::J, Modifiers::MAC_CMD, true, false)], mode),
            [Some("\x1b[106;9u".into())]
        );
    }

    #[test]
    fn legacy_events_are_unchanged_including_text_and_repeat() {
        let events = vec![
            key(Key::J, Modifiers::NONE, true, true),
            Event::Text("j".into()),
            key(Key::J, Modifiers::NONE, false, false),
            key(Key::I, Modifiers::CTRL, true, false),
            Event::Paste("hello".into()),
        ];
        let result = KeyboardEncoder::default().prepare(
            events.clone(),
            TermMode::ALT_SCREEN | TermMode::APP_CURSOR,
            unbound,
        );
        assert_eq!(
            result.iter().map(|e| &e.event).collect::<Vec<_>>(),
            events.iter().collect::<Vec<_>>()
        );
        assert!(result.iter().all(|e| e.encoding.is_none()));
    }

    use crate::bindings::{
        BindingAction, BindingsLayout, InputKind, KeyboardBinding,
    };
    use crate::TerminalMode;

    const EVENT_TYPES: TermMode =
        TermMode::DISAMBIGUATE_ESC_CODES.union(TermMode::REPORT_EVENT_TYPES);

    /// Runs one frame per event through the real binding table, so state
    /// carries across frames as it does in the view.
    fn frames(
        encoder: &mut KeyboardEncoder,
        layout: &BindingsLayout,
        screen: TermMode,
        steps: Vec<(Event, TermMode)>,
    ) -> Vec<Option<String>> {
        steps
            .into_iter()
            .map(|(event, flags)| {
                let mut out = encoder.prepare(vec![event], flags, |k, m| {
                    layout.scrolls_before_encoding(k, m, screen | flags)
                });
                assert_eq!(out.len(), 1);
                out.remove(0)
                    .encoding
                    .map(|v| String::from_utf8(v).unwrap())
            })
            .collect()
    }

    #[test]
    fn kitty_main_screen_scroll_bindings_win_but_byte_bindings_do_not() {
        // What egui-winit reports for the macOS Command key.
        let cmd = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        let layout = BindingsLayout::new();
        let mut encoder = KeyboardEncoder::default();
        let flags = TermMode::DISAMBIGUATE_ESC_CODES;
        let shift_page_up = key(Key::PageUp, Modifiers::SHIFT, true, false);
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (shift_page_up.clone(), flags),
                (key(Key::PageDown, Modifiers::SHIFT, true, false), flags),
                // Char binding (Ctrl+A => 0x01) and Esc binding
                // (Shift+Up => CSI 1;2A) keep the kitty encoding.
                (key(Key::A, Modifiers::CTRL, true, false), flags),
                (key(Key::PageUp, Modifiers::CTRL, true, false), flags),
                (key(Key::PageUp, cmd, true, false), flags),
            ],
        );
        assert_eq!(
            out,
            [
                None,
                None,
                Some("\x1b[97;5u".into()),
                Some("\x1b[5;5~".into()),
                Some("\x1b[5;9~".into()),
            ]
        );
        // Alternate screen: nothing scrolls ahead of the encoding, including
        // scroll bindings without an ALT_SCREEN exclusion (macOS Cmd+Home).
        let mut unrestricted = BindingsLayout::new();
        unrestricted.add_bindings(crate::generate_bindings!(
            KeyboardBinding;
            Home, Modifiers::COMMAND; BindingAction::ScrollToTop;
        ));
        assert!(unrestricted.scrolls_before_encoding(
            Key::Home,
            cmd,
            TermMode::empty()
        ));
        let out = frames(
            &mut KeyboardEncoder::default(),
            &unrestricted,
            TermMode::ALT_SCREEN,
            vec![
                (shift_page_up, flags),
                (key(Key::Home, cmd, true, false), flags),
            ],
        );
        assert_eq!(out, [Some("\x1b[5;2~".into()), Some("\x1b[1;9H".into())]);
    }

    #[test]
    fn kitty_hold_follows_the_press_owner_across_modifier_and_flag_changes() {
        let layout = BindingsLayout::new();
        let main = TermMode::empty();
        let ev =
            |mods, pressed, repeat| key(Key::PageUp, mods, pressed, repeat);
        let shift = Modifiers::SHIFT;
        let none = Modifiers::NONE;
        // Press went to the program: adding Shift mid-hold changes nothing.
        let out = frames(
            &mut KeyboardEncoder::default(),
            &layout,
            main,
            vec![
                (ev(none, true, false), EVENT_TYPES),
                (ev(shift, true, true), EVENT_TYPES),
                (ev(shift, false, false), EVENT_TYPES),
            ],
        );
        assert_eq!(
            out,
            [
                Some("\x1b[5;1:1~".into()),
                Some("\x1b[5;2:2~".into()),
                Some("\x1b[5;2:3~".into()),
            ]
        );
        // Press scrolled: repeats scroll while Shift is held, then nothing of
        // the hold reaches the program, even across a kitty flags change.
        let out = frames(
            &mut KeyboardEncoder::default(),
            &layout,
            main,
            vec![
                (ev(shift, true, false), EVENT_TYPES),
                (ev(shift, true, true), EVENT_TYPES),
                (ev(none, true, true), EVENT_TYPES),
                (
                    ev(none, false, false),
                    EVENT_TYPES | TermMode::REPORT_ALTERNATE_KEYS,
                ),
            ],
        );
        assert_eq!(out, [None, None, Some(String::new()), Some(String::new())]);
        // A lost release cannot capture the key: the next press decides anew.
        let out = frames(
            &mut KeyboardEncoder::default(),
            &layout,
            main,
            vec![
                (ev(shift, true, false), EVENT_TYPES),
                (ev(none, true, false), EVENT_TYPES),
                (ev(none, false, false), EVENT_TYPES),
            ],
        );
        assert_eq!(
            out,
            [None, Some("\x1b[5;1:1~".into()), Some("\x1b[5;1:3~".into())]
        );
    }

    #[test]
    fn program_hold_repeat_never_scrolls_after_kitty_turns_off() {
        let layout = BindingsLayout::new();
        let main = TermMode::empty();
        let legacy = TermMode::empty();
        let ev =
            |mods, pressed, repeat| key(Key::PageUp, mods, pressed, repeat);
        let shift = Modifiers::SHIFT;
        let none = Modifiers::NONE;
        // Program-owned kitty press, kitty turned off, Shift added: the repeat
        // still goes to the program (xterm modified form), never scrolls.
        let out = frames(
            &mut KeyboardEncoder::default(),
            &layout,
            main,
            vec![
                (ev(none, true, false), EVENT_TYPES),
                (ev(shift, true, true), legacy),
                (ev(shift, false, false), legacy),
            ],
        );
        assert_eq!(
            out,
            [Some("\x1b[5;1:1~".into()), Some("\x1b[5;2~".into()), None]
        );
    }

    #[test]
    fn release_held_ends_program_holds_once_and_swallows_late_releases() {
        let layout = BindingsLayout::new();
        let mut encoder = KeyboardEncoder::default();
        let ev = |k, mods, pressed| key(k, mods, pressed, false);
        // A program-owned hold and a scroll-owned hold.
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (ev(Key::PageDown, Modifiers::NONE, true), EVENT_TYPES),
                (ev(Key::PageUp, Modifiers::SHIFT, true), EVENT_TYPES),
            ],
        );
        assert_eq!(out, [Some("\x1b[6;1:1~".into()), None]);
        assert_eq!(encoder.release_held(EVENT_TYPES), b"\x1b[6;1:3~");
        assert!(encoder.release_held(EVENT_TYPES).is_empty());
        // The physical releases arrive later: neither reaches the program.
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (ev(Key::PageDown, Modifiers::NONE, false), EVENT_TYPES),
                (ev(Key::PageUp, Modifiers::SHIFT, false), EVENT_TYPES),
            ],
        );
        assert_eq!(out, [Some(String::new()), Some(String::new())]);
        // A modified printable hold is released with its press's modifiers;
        // without them it would not be reported at all.
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (ev(Key::J, Modifiers::CTRL, true), EVENT_TYPES),
                (ev(Key::K, Modifiers::ALT, true), EVENT_TYPES),
            ],
        );
        assert_eq!(
            out,
            [Some("\x1b[106;5:1u".into()), Some("\x1b[107;3:1u".into())]
        );
        // Hold order is not significant.
        let released =
            String::from_utf8(encoder.release_held(EVENT_TYPES)).unwrap();
        assert!(
            released.contains("\x1b[106;5:3u")
                && released.contains("\x1b[107;3:3u")
                && released.len() == "\x1b[106;5:3u\x1b[107;3:3u".len(),
            "{released:?}"
        );
        // Without release reporting, ending a hold writes nothing.
        frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![(
                ev(Key::PageDown, Modifiers::NONE, true),
                TermMode::DISAMBIGUATE_ESC_CODES,
            )],
        );
        assert!(encoder
            .release_held(TermMode::DISAMBIGUATE_ESC_CODES)
            .is_empty());
    }

    #[test]
    fn release_held_survives_a_flags_change_that_keeps_release_reporting() {
        let layout = BindingsLayout::new();
        let mut encoder = KeyboardEncoder::default();
        let seven = EVENT_TYPES | TermMode::REPORT_ALTERNATE_KEYS;
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (key(Key::PageUp, Modifiers::NONE, true, false), EVENT_TYPES),
                // Any later key under the new flags (CSI > 7 u).
                (key(Key::F5, Modifiers::NONE, true, false), seven),
            ],
        );
        assert_eq!(out[0].as_deref(), Some("\x1b[5;1:1~"));
        let released = String::from_utf8(encoder.release_held(seven)).unwrap();
        assert!(released.contains("\x1b[5;1:3~"), "{released:?}");
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![(key(Key::PageUp, Modifiers::NONE, false, false), seven)],
        );
        assert_eq!(out, [Some(String::new())], "late release swallowed");
    }

    #[test]
    fn release_held_keeps_the_press_modifiers_across_repeats() {
        let layout = BindingsLayout::new();
        let mut encoder = KeyboardEncoder::default();
        // Ctrl+J, Ctrl let go, J repeats unmodified (falls back to text).
        frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (key(Key::J, Modifiers::CTRL, true, false), EVENT_TYPES),
                (key(Key::J, Modifiers::NONE, true, true), EVENT_TYPES),
            ],
        );
        assert_eq!(encoder.release_held(EVENT_TYPES), b"\x1b[106;5:3u");
    }

    #[test]
    fn release_held_keeps_scroll_ownership_until_the_physical_release() {
        let layout = BindingsLayout::new();
        let mut encoder = KeyboardEncoder::default();
        let ev =
            |mods, pressed, repeat| key(Key::PageUp, mods, pressed, repeat);
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![(ev(Modifiers::SHIFT, true, false), EVENT_TYPES)],
        );
        assert_eq!(out, [None], "Shift+PageUp scrolls");
        assert!(encoder.release_held(EVENT_TYPES).is_empty());
        // Focus returns with PageUp still held; Shift is let go and PageUp
        // repeats: the hold still belongs to the scrollback.
        let out = frames(
            &mut encoder,
            &layout,
            TermMode::empty(),
            vec![
                (ev(Modifiers::NONE, true, true), EVENT_TYPES),
                (ev(Modifiers::NONE, false, false), EVENT_TYPES),
            ],
        );
        assert_eq!(out, [Some(String::new()), Some(String::new())]);
    }

    #[test]
    fn legacy_scroll_hold_release_is_swallowed_after_kitty_turns_on() {
        let layout = BindingsLayout::new();
        let ev = |pressed| key(Key::PageUp, Modifiers::SHIFT, pressed, false);
        // Legacy press that scrolled, event reporting turned on: the program
        // never sees its release.
        let out = frames(
            &mut KeyboardEncoder::default(),
            &layout,
            TermMode::empty(),
            vec![(ev(true), TermMode::empty()), (ev(false), EVENT_TYPES)],
        );
        assert_eq!(out, [None, Some(String::new())]);
    }
}
