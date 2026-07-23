//! Minimal Standard MIDI File parser for MIDI clip sources.
//!
//! Extracts note-on/note-off pairs from SMF format 0/1 files and rescales
//! their tick times to the model's [`plexi_daw_model::TICKS_PER_BEAT`]
//! resolution. Everything except notes (meta events, sysex, other channel
//! messages, embedded tempo — the model owns tempo) is skipped structurally.
//! SMPTE division and other formats are rejected with named errors.

use plexi_daw_model::TICKS_PER_BEAT;

use crate::engine::Note;

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("midi: truncated at byte {at}"))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("midi: truncated at byte {at}"))
}

/// Reads one variable-length quantity, returning `(value, next_pos)`.
fn read_varlen(bytes: &[u8], mut pos: usize) -> Result<(u64, usize), String> {
    let mut value: u64 = 0;
    for _ in 0..4 {
        let byte = *bytes
            .get(pos)
            .ok_or_else(|| format!("midi: truncated varlen at byte {pos}"))?;
        pos += 1;
        value = (value << 7) | u64::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
    }
    Err(format!("midi: varlen longer than 4 bytes at {pos}"))
}

/// Bounds-checks a declared event payload, returning the position after it.
/// A payload running past the track is malformed input, not end-of-track.
fn skip_payload(body: &[u8], at: usize, len: u64) -> Result<usize, String> {
    at.checked_add(len as usize)
        .filter(|&end| end <= body.len())
        .ok_or_else(|| format!("midi: event payload length {len} at byte {at} exceeds track"))
}

/// A note currently sounding while a track is scanned.
struct OpenNote {
    key: u8,
    channel: u8,
    velocity: u8,
    start_ticks: u64,
}

/// Parses an SMF byte stream into model-resolution notes, merged across all
/// tracks and sorted by `(start, key)` for deterministic downstream use.
/// Notes left open at end of track are closed at the track's final tick.
pub fn parse_smf(bytes: &[u8]) -> Result<Vec<Note>, String> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        return Err("midi: not an SMF stream (missing MThd)".to_string());
    }
    let header_len = read_u32(bytes, 4)?;
    if header_len < 6 {
        return Err(format!("midi: MThd length {header_len} < 6"));
    }
    let format = read_u16(bytes, 8)?;
    if format > 1 {
        return Err(format!("midi: unsupported SMF format {format} (only 0/1)"));
    }
    let division = read_u16(bytes, 12)?;
    if division & 0x8000 != 0 {
        return Err("midi: SMPTE division is unsupported".to_string());
    }
    if division == 0 {
        return Err("midi: zero ticks-per-quarter division".to_string());
    }
    let tpqn = u64::from(division);

    let mut notes: Vec<Note> = Vec::new();
    let mut pos = 8 + header_len as usize;
    while pos + 8 <= bytes.len() {
        if &bytes[pos..pos + 4] != b"MTrk" {
            return Err(format!("midi: expected MTrk chunk at byte {pos}"));
        }
        let track_len = read_u32(bytes, pos + 4)? as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(track_len)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| format!("midi: MTrk length {track_len} exceeds stream"))?;
        parse_track(&bytes[body_start..body_end], tpqn, &mut notes)?;
        pos = body_end;
    }
    notes.sort_by_key(|n| (n.start_ticks, n.key, n.length_ticks, n.velocity));
    Ok(notes)
}

/// Rescales source ticks (at `tpqn`) to model ticks (at `TICKS_PER_BEAT`).
fn rescale(ticks: u64, tpqn: u64) -> Result<u64, String> {
    ticks
        .checked_mul(TICKS_PER_BEAT)
        .map(|t| t / tpqn)
        .ok_or_else(|| format!("midi: tick value {ticks} overflows during rescale"))
}

fn parse_track(body: &[u8], tpqn: u64, notes: &mut Vec<Note>) -> Result<(), String> {
    let mut pos = 0usize;
    let mut abs_ticks: u64 = 0;
    let mut running_status: Option<u8> = None;
    let mut open: Vec<OpenNote> = Vec::new();

    let close = |open: &mut Vec<OpenNote>, key: u8, channel: u8, end_ticks: u64, notes: &mut Vec<Note>| -> Result<(), String> {
        if let Some(i) = open.iter().position(|n| n.key == key && n.channel == channel) {
            let n = open.remove(i);
            let start = rescale(n.start_ticks, tpqn)?;
            let end = rescale(end_ticks, tpqn)?;
            if end > start {
                notes.push(Note {
                    key: n.key,
                    velocity: n.velocity,
                    start_ticks: start,
                    length_ticks: end - start,
                });
            }
        }
        Ok(())
    };

    while pos < body.len() {
        let (delta, next) = read_varlen(body, pos)?;
        pos = next;
        abs_ticks = abs_ticks
            .checked_add(delta)
            .ok_or_else(|| "midi: absolute tick overflow".to_string())?;
        let first = *body
            .get(pos)
            .ok_or_else(|| format!("midi: truncated event at byte {pos}"))?;
        let status = if first & 0x80 != 0 {
            pos += 1;
            if first < 0xF0 {
                running_status = Some(first);
            }
            first
        } else {
            running_status.ok_or_else(|| format!("midi: data byte {first:#04x} with no running status"))?
        };
        match status {
            0xFF => {
                // Meta event: type byte + varlen + payload.
                pos += 1; // type
                let (len, next) = read_varlen(body, pos)?;
                pos = skip_payload(body, next, len)?;
            }
            0xF0 | 0xF7 => {
                let (len, next) = read_varlen(body, pos)?;
                pos = skip_payload(body, next, len)?;
            }
            _ => {
                let kind = status & 0xF0;
                let channel = status & 0x0F;
                let data_len = match kind {
                    0xC0 | 0xD0 => 1,
                    _ => 2,
                };
                let data = body
                    .get(pos..pos + data_len)
                    .ok_or_else(|| format!("midi: truncated channel event at byte {pos}"))?;
                pos += data_len;
                match kind {
                    0x90 if data[1] > 0 => open.push(OpenNote {
                        key: data[0],
                        channel,
                        velocity: data[1],
                        start_ticks: abs_ticks,
                    }),
                    0x90 | 0x80 => close(&mut open, data[0], channel, abs_ticks, notes)?,
                    _ => {}
                }
            }
        }
    }
    // Close anything still sounding at end of track.
    while let Some(n) = open.first() {
        let (key, channel) = (n.key, n.channel);
        close(&mut open, key, channel, abs_ticks, notes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a single-track SMF with the given events (delta, bytes).
    fn smf(tpqn: u16, events: &[(u64, &[u8])]) -> Vec<u8> {
        let mut track = Vec::new();
        for (delta, bytes) in events {
            let mut d = *delta;
            let mut var = vec![(d & 0x7F) as u8];
            d >>= 7;
            while d > 0 {
                var.push(((d & 0x7F) as u8) | 0x80);
                d >>= 7;
            }
            var.reverse();
            track.extend_from_slice(&var);
            track.extend_from_slice(bytes);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&tpqn.to_be_bytes());
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&(track.len() as u32).to_be_bytes());
        out.extend_from_slice(&track);
        out
    }

    #[test]
    fn parses_and_rescales_notes() {
        // 480 tpqn source: quarter note C4 at beat 0, then E4 eighth at beat 1.
        let bytes = smf(
            480,
            &[
                (0, &[0x90, 60, 100]),
                (480, &[0x80, 60, 0]),
                (0, &[0x90, 64, 80]),
                (240, &[0x80, 64, 0]),
            ],
        );
        let notes = parse_smf(&bytes).unwrap();
        assert_eq!(
            notes,
            vec![
                Note { key: 60, velocity: 100, start_ticks: 0, length_ticks: 960 },
                Note { key: 64, velocity: 80, start_ticks: 960, length_ticks: 480 },
            ]
        );
    }

    #[test]
    fn running_status_and_velocity_zero_note_off() {
        // Note-on then note-off via running status + velocity 0.
        let bytes = smf(960, &[(0, &[0x90, 72, 90]), (960, &[72, 0])]);
        let notes = parse_smf(&bytes).unwrap();
        assert_eq!(
            notes,
            vec![Note { key: 72, velocity: 90, start_ticks: 0, length_ticks: 960 }]
        );
    }

    #[test]
    fn open_note_closes_at_track_end() {
        let bytes = smf(960, &[(0, &[0x90, 60, 64]), (480, &[0xFF, 0x2F, 0x00])]);
        let notes = parse_smf(&bytes).unwrap();
        assert_eq!(
            notes,
            vec![Note { key: 60, velocity: 64, start_ticks: 0, length_ticks: 480 }]
        );
    }

    #[test]
    fn rejects_truncated_meta_payload() {
        // Meta event declaring 100 payload bytes inside a track that ends
        // immediately after the declaration.
        let bytes = smf(480, &[(0, &[0xFF, 0x01, 100])]);
        let err = parse_smf(&bytes).unwrap_err();
        assert!(err.contains("exceeds track"), "{err}");
    }

    #[test]
    fn rejects_smpte_and_non_smf() {
        assert!(parse_smf(b"junk").unwrap_err().contains("MThd"));
        let mut bytes = smf(480, &[]);
        // Flip division to SMPTE.
        bytes[12] = 0xE8;
        assert!(parse_smf(&bytes).unwrap_err().contains("SMPTE"));
    }
}
