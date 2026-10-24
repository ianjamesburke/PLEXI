//! OSC 7501 byte scanner and report parser.
use super::records::{Kind, Record, Report, State};
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::{alphabet, Engine};

/// Whole sequence, OSC through ST.
pub const MAX_SEQUENCE: usize = 4096;
const KEY_MAX: usize = 16;
const MSG_ENCODED_MAX: usize = 2732;
const MSG_DECODED_MAX: usize = 2048;
const TITLE_ENCODED_MAX: usize = 256;
const TITLE_DECODED_MAX: usize = 192;
const APP_MAX: usize = 32;
const ID_MAX: usize = 128;
const SEGMENT_MAX: usize = 32;
const ID_DEPTH_MAX: usize = 8;

/// Exact feature-detection reply (`?` query).
pub const FEATURE_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// Raw payload of a complete, in-limit `OSC 7501 ; payload` sequence.
    Report(Vec<u8>),
    /// `OSC 7501 ; ?`
    Query,
    /// `OSC 133 ; A`
    PromptStart,
    /// `ESC c`
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ground,
    Esc,
    OscHead,
    Payload,
    Osc133,
    OscOther,
}

/// Observe-only state machine over PTY bytes; state persists across reads.
#[derive(Debug)]
pub struct Osc7501Scanner {
    mode: Mode,
    esc_in_osc: bool,
    num: u32,
    num_len: usize,
    buf: Vec<u8>,
    overflow: bool,
    seq_len: usize,
    p133: [Option<u8>; 2],
    p133_len: usize,
}

impl Default for Osc7501Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Osc7501Scanner {
    pub fn new() -> Self {
        Self {
            mode: Mode::Ground,
            esc_in_osc: false,
            num: 0,
            num_len: 0,
            buf: Vec::new(),
            overflow: false,
            seq_len: 0,
            p133: [None; 2],
            p133_len: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if self.mode == Mode::Ground {
                match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(p) => {
                        i += p;
                        self.mode = Mode::Esc;
                        i += 1;
                        continue;
                    }
                    None => break,
                }
            }
            self.step(bytes[i], &mut out);
            i += 1;
        }
        out
    }

    fn in_osc(&self) -> bool {
        !matches!(self.mode, Mode::Ground | Mode::Esc)
    }

    fn start_osc(&mut self) {
        self.mode = Mode::OscHead;
        self.esc_in_osc = false;
        self.num = 0;
        self.num_len = 0;
        self.buf.clear();
        self.overflow = false;
        self.seq_len = 2;
        self.p133 = [None; 2];
        self.p133_len = 0;
    }

    fn abort_osc(&mut self) {
        self.mode = Mode::Ground;
        self.esc_in_osc = false;
        self.buf.clear();
    }

    fn step(&mut self, b: u8, out: &mut Vec<ScanEvent>) {
        if self.in_osc() {
            self.seq_len = self.seq_len.saturating_add(1);
            if self.esc_in_osc {
                self.esc_in_osc = false;
                if b == b'\\' {
                    self.finish(out);
                    return;
                }
                // ESC not followed by `\` ends the string; `b` is the byte after ESC.
                self.abort_osc();
                self.mode = Mode::Esc;
            }
        }
        match self.mode {
            Mode::Ground => {
                if b == ESC {
                    self.mode = Mode::Esc;
                }
            }
            Mode::Esc => match b {
                b']' => self.start_osc(),
                b'c' => {
                    out.push(ScanEvent::Reset);
                    self.mode = Mode::Ground;
                }
                ESC => {}
                _ => self.mode = Mode::Ground,
            },
            _ => match b {
                BEL => self.finish(out),
                ESC => self.esc_in_osc = true,
                CAN | SUB => self.abort_osc(),
                _ => self.osc_byte(b),
            },
        }
    }

    fn osc_byte(&mut self, b: u8) {
        match self.mode {
            Mode::OscHead => {
                if b.is_ascii_digit() {
                    self.num = self.num.saturating_mul(10).saturating_add((b - b'0') as u32);
                    self.num_len += 1;
                    if self.num_len > 5 {
                        self.mode = Mode::OscOther;
                    }
                } else if b == b';' {
                    self.mode = match (self.num, self.num_len) {
                        (7501, 4) => Mode::Payload,
                        (133, 3) => Mode::Osc133,
                        _ => Mode::OscOther,
                    };
                } else {
                    self.mode = Mode::OscOther;
                }
            }
            Mode::Payload => {
                if self.seq_len > MAX_SEQUENCE {
                    self.overflow = true;
                    self.buf.clear();
                } else if !self.overflow {
                    self.buf.push(b);
                }
            }
            Mode::Osc133 if self.p133_len < 2 => {
                self.p133[self.p133_len] = Some(b);
                self.p133_len += 1;
            }
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut Vec<ScanEvent>) {
        match self.mode {
            Mode::Payload if !self.overflow && self.seq_len <= MAX_SEQUENCE => {
                let payload = std::mem::take(&mut self.buf);
                if payload == b"?" {
                    out.push(ScanEvent::Query);
                } else {
                    out.push(ScanEvent::Report(payload));
                }
            }
            Mode::Osc133
                if self.p133[0] == Some(b'A')
                    && matches!(self.p133[1], None | Some(b';')) =>
            {
                out.push(ScanEvent::PromptStart);
            }
            _ => {}
        }
        self.abort_osc();
    }
}

fn value_ok(v: &[u8]) -> bool {
    v.iter().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
    })
}

fn b64() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    )
}

/// `None` discards the whole report.
fn decode_text(val: &[u8], encoded_max: usize, decoded_max: usize) -> Option<String> {
    if val.len() > encoded_max {
        return None;
    }
    let bytes = b64().decode(val).ok()?;
    if bytes.len() > decoded_max {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(text)
}

fn segment_ok(seg: &[u8]) -> bool {
    !seg.is_empty()
        && seg.len() <= SEGMENT_MAX
        && seg
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'+' | b'-'))
}

/// `None` is a bad id. Absent id is the root record (empty string).
fn parse_id(v: Option<&[u8]>) -> Option<String> {
    let Some(v) = v else {
        return Some(String::new());
    };
    if v.is_empty() || v.len() > ID_MAX {
        return None;
    }
    let mut depth = 0;
    for seg in v.split(|&b| b == b'/') {
        depth += 1;
        if depth > ID_DEPTH_MAX || !segment_ok(seg) {
            return None;
        }
    }
    Some(String::from_utf8_lossy(v).into_owned())
}

fn parse_progress(v: &[u8]) -> Option<u8> {
    if v.is_empty() || v.len() > 3 || !v.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n: u32 = std::str::from_utf8(v).ok()?.parse().ok()?;
    u8::try_from(n).ok().filter(|n| *n <= 100)
}

/// Parse one report payload (the bytes after `OSC 7501 ;`). `None` means the
/// report is discarded or ignored; nothing from it may be applied.
pub fn parse_report(payload: &[u8]) -> Option<Report> {
    let mut state_v: Option<&[u8]> = None;
    let mut id_v: Option<&[u8]> = None;
    let mut kind_v: Option<&[u8]> = None;
    let mut progress_v: Option<&[u8]> = None;
    let mut app_v: Option<&[u8]> = None;
    let mut msg: Option<String> = None;
    let mut title: Option<String> = None;

    for pair in payload.split(|&b| b == b':') {
        let Some(eq) = pair.iter().position(|&b| b == b'=') else {
            continue;
        };
        let key = pair[..eq].trim_ascii();
        let val = pair[eq + 1..].trim_ascii();
        // A bad id must ignore the whole report, never fall back to the root record.
        if key == b"id" && !value_ok(val) {
            return None;
        }
        if key.is_empty() || !key.iter().all(u8::is_ascii_lowercase) || !value_ok(val) {
            continue;
        }
        if key.len() > KEY_MAX {
            return None;
        }
        match key {
            b"state" => state_v = Some(val),
            b"id" => id_v = Some(val),
            b"kind" => kind_v = Some(val),
            b"progress" => progress_v = Some(val),
            b"app" => app_v = Some(val),
            b"msg" => msg = Some(decode_text(val, MSG_ENCODED_MAX, MSG_DECODED_MAX)?),
            b"title" => title = Some(decode_text(val, TITLE_ENCODED_MAX, TITLE_DECODED_MAX)?),
            _ => {}
        }
    }

    let state_raw = state_v?;
    if state_raw == b"clear" {
        return Some(Report::Clear { id: parse_id(id_v)? });
    }
    let state = match state_raw {
        b"idle" => State::Idle,
        b"working" => State::Working,
        b"done" => State::Done,
        b"blocked" => State::Blocked,
        b"error" => State::Error,
        _ => return None,
    };
    let id = parse_id(id_v)?;
    let kind = match (state, kind_v) {
        (State::Blocked, Some(b"permission")) => Some(Kind::Permission),
        (State::Blocked, Some(b"question")) => Some(Kind::Question),
        (State::Blocked, Some(b"auth")) => Some(Kind::Auth),
        _ => None,
    };
    let progress = match state {
        State::Working | State::Blocked => progress_v.and_then(parse_progress),
        _ => None,
    };
    let app = app_v
        .filter(|a| segment_ok(a) && a.len() <= APP_MAX)
        .map(|a| String::from_utf8_lossy(a).into_owned());
    Some(Report::Set {
        id,
        record: Record {
            state,
            kind,
            progress,
            msg: msg.filter(|m| !m.is_empty()),
            title: title.filter(|t| !t.is_empty()),
            app,
            seen: false,
            updated_at: 0,
        },
    })
}
