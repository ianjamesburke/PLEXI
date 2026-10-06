//! Permission MAC for the Needs you journal.
//!
//! This is the slice of the host seal the journal needs. The permission-seal
//! crate (#2718) replaces the module. `existing_mac_key` reads and does not
//! create. `mac_key_bytes` creates the MAC on first use.

use zeroize::Zeroizing;

/// Permission MAC if the host seal store already has one. Does not create a key.
pub(crate) fn existing_mac_key() -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    super::host_key::scrub_user_secret_host_namespace();
    match super::host_key::get(super::host_key::MAC_ITEM)? {
        Some(existing) => Ok(Some(Zeroizing::new(decode_hex(existing.trim())?))),
        None => Ok(None),
    }
}

/// Permission MAC, creating it in the host seal store when this is the first use.
pub(crate) fn mac_key_bytes() -> Result<Zeroizing<Vec<u8>>, String> {
    let hex = key_hex()?;
    Ok(Zeroizing::new(decode_hex(hex.as_str())?))
}

fn key_hex() -> Result<Zeroizing<String>, String> {
    super::host_key::scrub_user_secret_host_namespace();
    if let Some(existing) = super::host_key::get(super::host_key::MAC_ITEM)? {
        decode_hex(existing.trim())?;
        return Ok(Zeroizing::new(existing.trim().to_string()));
    }
    let hex = fresh_key_hex();
    match super::host_key::add_new(super::host_key::MAC_ITEM, &hex) {
        Ok(()) => {
            log::info!("permission_seal: created host permission mac key");
            Ok(Zeroizing::new(hex))
        }
        Err(error) => match super::host_key::get(super::host_key::MAC_ITEM)? {
            Some(existing) => {
                let trimmed = existing.trim().to_string();
                decode_hex(&trimmed)?;
                Ok(Zeroizing::new(trimmed))
            }
            None => Err(error),
        },
    }
}

fn fresh_key_hex() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("host seal key is not hex".to_string());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let hi = from_hex(bytes[index])?;
        let lo = from_hex(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    if out.is_empty() {
        return Err("host seal key is empty".to_string());
    }
    Ok(out)
}

fn from_hex(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("host seal key is not hex".to_string()),
    }
}
