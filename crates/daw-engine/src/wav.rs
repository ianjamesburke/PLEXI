//! Minimal WAV codec for clip sources and mixdown export.
//!
//! Decodes 16-bit PCM (format 1) and 32-bit IEEE float (format 3) RIFF/WAVE
//! files into interleaved f32; encodes interleaved f32 back out as format 3.
//! Anything else is rejected with an error naming what was unsupported —
//! compressed formats are explicitly out of scope for v1.

/// Decoded WAV audio: interleaved f32 samples in `[-1.0, 1.0]` (PCM16 is
/// scaled by `1/32768`; float data is taken as-is).
#[derive(Debug, Clone, PartialEq)]
pub struct WavData {
    pub sample_rate: u32,
    pub channels: u32,
    /// Interleaved; length is a multiple of `channels`.
    pub samples: Vec<f32>,
}

impl WavData {
    #[must_use]
    pub fn frames(&self) -> u64 {
        if self.channels == 0 {
            0
        } else {
            (self.samples.len() / self.channels as usize) as u64
        }
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("wav: truncated at byte {at} reading u16"))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("wav: truncated at byte {at} reading u32"))
}

/// Decodes a RIFF/WAVE byte stream. Errors name the offending chunk, format
/// code, or bit depth.
pub fn decode(bytes: &[u8]) -> Result<WavData, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("wav: not a RIFF/WAVE stream".to_string());
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u32, u32, u16)> = None; // (format, channels, rate, bits)
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let chunk_len = read_u32(bytes, pos + 4)? as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(chunk_len)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| {
                format!("wav: chunk {:?} length {chunk_len} exceeds stream", String::from_utf8_lossy(id))
            })?;
        match id {
            b"fmt " => {
                if chunk_len < 16 {
                    return Err(format!("wav: fmt chunk too short ({chunk_len} bytes)"));
                }
                let format = read_u16(bytes, body_start)?;
                let channels = u32::from(read_u16(bytes, body_start + 2)?);
                let rate = read_u32(bytes, body_start + 4)?;
                let bits = read_u16(bytes, body_start + 14)?;
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        // Chunks are word-aligned: odd lengths carry one pad byte.
        pos = body_end + (chunk_len & 1);
    }
    let (format, channels, sample_rate, bits) =
        fmt.ok_or_else(|| "wav: missing fmt chunk".to_string())?;
    let data = data.ok_or_else(|| "wav: missing data chunk".to_string())?;
    if channels == 0 {
        return Err("wav: fmt declares zero channels".to_string());
    }
    if sample_rate == 0 {
        return Err("wav: fmt declares zero sample rate".to_string());
    }
    let samples = match (format, bits) {
        (1, 16) => data
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect::<Vec<f32>>(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect::<Vec<f32>>(),
        (format, bits) => {
            return Err(format!(
                "wav: unsupported format code {format} / {bits}-bit (only PCM16 and float32)"
            ));
        }
    };
    let trimmed = samples.len() - samples.len() % channels as usize;
    Ok(WavData {
        sample_rate,
        channels,
        samples: {
            let mut s = samples;
            s.truncate(trimmed);
            s
        },
    })
}

/// Encodes interleaved f32 samples as a 32-bit IEEE float WAV (format 3).
/// This is the mixdown/export substrate: same samples in, byte-identical
/// file out.
pub fn encode_f32(sample_rate: u32, channels: u32, samples: &[f32]) -> Result<Vec<u8>, String> {
    if channels == 0 {
        return Err("wav encode: zero channels".to_string());
    }
    if sample_rate == 0 {
        return Err("wav encode: zero sample rate".to_string());
    }
    if !samples.len().is_multiple_of(channels as usize) {
        return Err(format!(
            "wav encode: sample count {} is not a multiple of {channels} channels",
            samples.len()
        ));
    }
    let data_len = u32::try_from(samples.len() * 4)
        .map_err(|_| "wav encode: data exceeds u32 byte length".to_string())?;
    let byte_rate = sample_rate
        .checked_mul(channels * 4)
        .ok_or_else(|| "wav encode: byte rate overflows u32".to_string())?;
    let mut out = Vec::with_capacity(44 + samples.len() * 4);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&((channels * 4) as u16).to_le_bytes()); // block align
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_round_trip_is_lossless() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0, 0.25];
        let bytes = encode_f32(48_000, 2, &samples).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples, samples);
        assert_eq!(decoded.frames(), 3);
    }

    #[test]
    fn pcm16_decodes_scaled() {
        // Hand-built mono PCM16 WAV: samples [0, 16384, -32768].
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36u32 + 6).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&6u32.to_le_bytes());
        for v in [0i16, 16384, -32768] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.sample_rate, 8_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn rejects_non_wav_and_unsupported_formats() {
        assert!(decode(b"nope").unwrap_err().contains("RIFF"));
        // 8-bit PCM is unsupported.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(decode(&bytes).unwrap_err().contains("unsupported format"));
    }

    #[test]
    fn truncated_chunk_is_an_error() {
        let good = encode_f32(8_000, 1, &[0.1, 0.2, 0.3]).unwrap();
        let err = decode(&good[..good.len() - 2]).unwrap_err();
        assert!(err.contains("exceeds stream"), "{err}");
    }
}
