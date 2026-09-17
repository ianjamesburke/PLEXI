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

#[derive(Clone, Default)]
pub(crate) struct KeyboardEncoder {
    held: HashMap<Key, Code>,
    mode: TermMode,
}

pub(crate) struct PreparedEvent {
    pub event: Event,
    /// None delegates to the existing legacy binding path; an empty encoding
    /// consumes the event without writing (e.g. release reporting is disabled).
    pub encoding: Option<Vec<u8>>,
}

impl KeyboardEncoder {
    pub fn prepare(
        &mut self,
        events: Vec<Event>,
        mode: TermMode,
    ) -> Vec<PreparedEvent> {
        let mode = mode & TermMode::KITTY_KEYBOARD_PROTOCOL;
        if self.mode != mode {
            self.held.clear();
            self.mode = mode;
        }
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
                } if !mode.is_empty() => {
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
                    let identity = physical_key.unwrap_or(*key);
                    let code = if !pressed {
                        self.held
                            .remove(&identity)
                            .or_else(|| key_code(*key, *modifiers))
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
                        if let Some(code) = code {
                            self.held.insert(identity, code);
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
    if !all && matches!(code.number, 9 | 13 | 127) {
        return if pressed { None } else { Some(Vec::new()) };
    }
    let chord = mods.ctrl || mods.alt || mods.mac_cmd;
    let encode = all
        || (code.text && disambiguate && chord)
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

    fn encoded(events: Vec<Event>, mode: TermMode) -> Vec<Option<String>> {
        KeyboardEncoder::default()
            .prepare(events, mode)
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
                encoded(vec![key(k, Modifiers::NONE, false, false)], mode),
                [Some(expected.into())]
            );
        }
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
        );
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].encoding.as_deref(),
            Some(b"\x1b[1078;2:1;1046u".as_slice())
        );
        // The release may have no text and a different modifier state.
        let result = encoder
            .prepare(vec![key(Key::A, Modifiers::NONE, false, false)], mode);
        assert_eq!(
            result[0].encoding.as_deref(),
            Some(b"\x1b[1078;1:3u".as_slice())
        );
        assert_eq!(
            encoded(vec![Event::Text("日本".into())], mode),
            [Some("\x1b[0;1;26085:26412u".into())]
        );
        let result =
            encoder.prepare(vec![Event::Paste("paste me".into())], mode);
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
        );
        assert_eq!(
            result.iter().map(|e| &e.event).collect::<Vec<_>>(),
            events.iter().collect::<Vec<_>>()
        );
        assert!(result.iter().all(|e| e.encoding.is_none()));
    }
}
