//! Phone-desktop seals.
//!
//! The desktop public key is shown in the pairing URL fragment. The relay
//! never receives it. Each body is ChaCha20-Poly1305 under a key from
//! X25519 and HKDF-SHA256. Counters reject a replay. `ring` provides the
//! primitives; this module only lays out the envelope.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use x25519_dalek::{PublicKey, StaticSecret};

const MAGIC: &[u8] = b"P1";
const INFO: &[u8] = b"plexi-phone-e2e-v1";
const KIND_HANDSHAKE: u8 = 0x01;
const KIND_DATA: u8 = 0x02;
const DIR_PHONE: u8 = 1;
const DIR_DESKTOP: u8 = 2;
const PUB_LEN: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SealError {
    Malformed,
    Tamper,
    Replay,
    NoKey,
}

pub(crate) struct PendingDesktopKey {
    secret: StaticSecret,
    pub public: [u8; PUB_LEN],
}

pub(crate) struct PhoneSession {
    key: [u8; 32],
    send: u64,
    recv: u64,
    desktop_pub: [u8; PUB_LEN],
    phone_pub: [u8; PUB_LEN],
}

#[derive(Debug)]
pub(crate) struct Opened {
    pub text: String,
    pub join_desktop: bool,
}

#[derive(Debug)]
pub(crate) enum PhoneOpen {
    New {
        opened: Opened,
        session: PhoneSession,
    },
    Existing {
        opened: Opened,
    },
}

pub(crate) fn generate_desktop_key() -> Result<PendingDesktopKey, SealError> {
    let mut bytes = [0u8; PUB_LEN];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SealError::NoKey)?;
    Ok(desktop_key_from_bytes(bytes))
}

pub(crate) fn desktop_key_from_bytes(bytes: [u8; PUB_LEN]) -> PendingDesktopKey {
    let secret = StaticSecret::from(bytes);
    let public = PublicKey::from(&secret).to_bytes();
    PendingDesktopKey { secret, public }
}

pub(crate) fn desktop_key_record(key: &PendingDesktopKey) -> String {
    URL_SAFE_NO_PAD.encode(key.secret.to_bytes())
}

pub(crate) fn desktop_key_from_record(text: &str) -> Option<PendingDesktopKey> {
    let bytes: [u8; PUB_LEN] = decode_array(text.trim())?;
    Some(desktop_key_from_bytes(bytes))
}

pub(crate) fn key_fingerprint(public: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, public);
    digest.as_ref()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn qr_with_public_key(qr_url: &str, public: &[u8; PUB_LEN]) -> String {
    let base = qr_url.split('#').next().unwrap_or(qr_url);
    format!("{base}#k={}", URL_SAFE_NO_PAD.encode(public))
}

pub(crate) fn open_phone_body(
    body_b64: &str,
    request_id: &str,
    pending: &mut Option<PendingDesktopKey>,
    session: Option<&mut PhoneSession>,
) -> Result<PhoneOpen, SealError> {
    let raw = URL_SAFE_NO_PAD
        .decode(body_b64.trim())
        .map_err(|_| SealError::Malformed)?;
    if raw.len() < 3 || &raw[..2] != MAGIC {
        return Err(SealError::Malformed);
    }
    match raw[2] {
        KIND_HANDSHAKE => {
            if session.is_some() {
                return Err(SealError::Replay);
            }
            open_handshake(&raw, pending)
        }
        KIND_DATA => {
            let session = session.ok_or(SealError::NoKey)?;
            let opened = open_data_phone(&raw, request_id, session)?;
            Ok(PhoneOpen::Existing { opened })
        }
        _ => Err(SealError::Malformed),
    }
}

pub(crate) fn seal_reply(
    session: &mut PhoneSession,
    text: &str,
    error: Option<&str>,
    request_id: &str,
) -> Result<String, SealError> {
    let counter = session.send.checked_add(1).ok_or(SealError::Malformed)?;
    let raw = seal_data(
        &session.key,
        DIR_DESKTOP,
        counter,
        request_id,
        text,
        false,
        error,
    )?;
    session.send = counter;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

impl std::fmt::Debug for PhoneSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhoneSession")
            .field("send", &self.send)
            .field("recv", &self.recv)
            .finish()
    }
}

impl PhoneSession {
    pub(crate) fn to_record(&self) -> String {
        json!({
            "v": 1,
            "key": URL_SAFE_NO_PAD.encode(self.key),
            "send": self.send,
            "recv": self.recv,
            "desk": URL_SAFE_NO_PAD.encode(self.desktop_pub),
            "phone": URL_SAFE_NO_PAD.encode(self.phone_pub),
        })
        .to_string()
    }

    pub(crate) fn from_record(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        if value.get("v").and_then(|item| item.as_u64()) != Some(1) {
            return None;
        }
        Some(Self {
            key: decode_array(value.get("key")?.as_str()?)?,
            send: value.get("send").and_then(|item| item.as_u64())?,
            recv: value.get("recv").and_then(|item| item.as_u64())?,
            desktop_pub: decode_array(value.get("desk")?.as_str()?)?,
            phone_pub: decode_array(value.get("phone")?.as_str()?)?,
        })
    }
}

fn open_handshake(raw: &[u8], pending: &mut Option<PendingDesktopKey>) -> Result<PhoneOpen, SealError> {
    if raw.len() < 3 + PUB_LEN + 16 {
        return Err(SealError::Malformed);
    }
    let phone_pub: [u8; PUB_LEN] = raw[3..3 + PUB_LEN]
        .try_into()
        .map_err(|_| SealError::Malformed)?;
    let pending_key = pending.as_ref().ok_or(SealError::NoKey)?;
    let desktop_pub = pending_key.public;
    // The desktop key stays. A second phone, and a restart before the first
    // message, both need the same private key. A bad handshake does not burn it.
    let shared = pending_key
        .secret
        .diffie_hellman(&PublicKey::from(phone_pub));
    let key = derive_session_key(shared.as_bytes(), &desktop_pub, &phone_pub)?;
    let aad = aad_handshake(&phone_pub, &desktop_pub);
    let plain = open_aead(&key, nonce(DIR_PHONE, 0)?, &aad, &raw[3 + PUB_LEN..])?;
    let opened = parse_plain(&plain)?;
    Ok(PhoneOpen::New {
        opened,
        session: PhoneSession {
            key,
            send: 0,
            recv: 0,
            desktop_pub,
            phone_pub,
        },
    })
}

fn open_data_phone(raw: &[u8], request_id: &str, session: &mut PhoneSession) -> Result<Opened, SealError> {
    let (direction, counter, ciphertext) = parse_data(raw)?;
    if direction != DIR_PHONE {
        return Err(SealError::Tamper);
    }
    if counter == 0 || counter <= session.recv {
        return Err(SealError::Replay);
    }
    let aad = aad_data(direction, counter, request_id);
    let plain = open_aead(&session.key, nonce(direction, counter)?, &aad, ciphertext)?;
    let opened = parse_plain(&plain)?;
    session.recv = counter;
    Ok(opened)
}

fn parse_data(raw: &[u8]) -> Result<(u8, u64, &[u8]), SealError> {
    if raw.len() < 12 + 16 || &raw[..2] != MAGIC || raw[2] != KIND_DATA {
        return Err(SealError::Malformed);
    }
    let direction = raw[3];
    let counter = u64::from_le_bytes(raw[4..12].try_into().map_err(|_| SealError::Malformed)?);
    Ok((direction, counter, &raw[12..]))
}

fn seal_data(
    key: &[u8; 32],
    direction: u8,
    counter: u64,
    request_id: &str,
    text: &str,
    join_desktop: bool,
    error: Option<&str>,
) -> Result<Vec<u8>, SealError> {
    if counter == 0 {
        return Err(SealError::Replay);
    }
    let aad = aad_data(direction, counter, request_id);
    let mut body = seal_aead(key, nonce(direction, counter)?, &aad, &plain_bytes(text, join_desktop, error))?;
    let mut raw = Vec::with_capacity(12 + body.len());
    raw.extend_from_slice(MAGIC);
    raw.push(KIND_DATA);
    raw.push(direction);
    raw.extend_from_slice(&counter.to_le_bytes());
    raw.append(&mut body);
    Ok(raw)
}

fn derive_session_key(
    shared: &[u8],
    desktop_pub: &[u8; PUB_LEN],
    phone_pub: &[u8; PUB_LEN],
) -> Result<[u8; 32], SealError> {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(desktop_pub);
    salt[32..].copy_from_slice(phone_pub);
    let hk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, &salt);
    let prk = hk.extract(shared);
    let okm = prk
        .expand(&[INFO], &CHACHA20_POLY1305)
        .map_err(|_| SealError::Tamper)?;
    let mut out = [0u8; 32];
    okm.fill(&mut out).map_err(|_| SealError::Tamper)?;
    Ok(out)
}

fn nonce(direction: u8, counter: u64) -> Result<Nonce, SealError> {
    let mut bytes = [0u8; 12];
    bytes[0] = direction;
    bytes[4..].copy_from_slice(&counter.to_le_bytes());
    Ok(Nonce::assume_unique_for_key(bytes))
}

fn aad_handshake(phone_pub: &[u8; PUB_LEN], desktop_pub: &[u8; PUB_LEN]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(2 + 1 + 64);
    aad.extend_from_slice(MAGIC);
    aad.push(KIND_HANDSHAKE);
    aad.extend_from_slice(phone_pub);
    aad.extend_from_slice(desktop_pub);
    aad
}

fn aad_data(direction: u8, counter: u64, request_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(12 + request_id.len());
    aad.extend_from_slice(MAGIC);
    aad.push(KIND_DATA);
    aad.push(direction);
    aad.extend_from_slice(&counter.to_le_bytes());
    aad.extend_from_slice(request_id.as_bytes());
    aad
}

fn plain_bytes(text: &str, join_desktop: bool, error: Option<&str>) -> Vec<u8> {
    let mut payload = json!({
        "v": 1,
        "text": text,
        "join_desktop": join_desktop,
    });
    if let Some(error) = error {
        payload["error"] = json!(error);
    }
    payload.to_string().into_bytes()
}

fn parse_plain(raw: &[u8]) -> Result<Opened, SealError> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| SealError::Tamper)?;
    if value.get("v").and_then(|item| item.as_u64()) != Some(1) {
        return Err(SealError::Tamper);
    }
    let text = value
        .get("text")
        .and_then(|item| item.as_str())
        .ok_or(SealError::Tamper)?
        .to_string();
    let join_desktop = value
        .get("join_desktop")
        .and_then(|item| item.as_bool())
        .ok_or(SealError::Tamper)?;
    Ok(Opened { text, join_desktop })
}

fn seal_aead(key: &[u8; 32], nonce: Nonce, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).map_err(|_| SealError::Tamper)?;
    let aead = LessSafeKey::new(unbound);
    let mut in_out = plaintext.to_vec();
    aead.seal_in_place_append_tag(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| SealError::Tamper)?;
    Ok(in_out)
}

fn open_aead(key: &[u8; 32], nonce: Nonce, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, SealError> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).map_err(|_| SealError::Tamper)?;
    let aead = LessSafeKey::new(unbound);
    let mut in_out = ciphertext.to_vec();
    let plain = aead
        .open_in_place(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| SealError::Tamper)?;
    Ok(plain.to_vec())
}

fn decode_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python_phone(pending: &PendingDesktopKey) -> Value {
        let output = std::process::Command::new("python3")
            .args([
                "services/relay/phone_crypto.py",
                "handshake-for",
                &URL_SAFE_NO_PAD.encode(pending.public),
            ])
            .output()
            .expect("python phone_crypto");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn unwrap_new(opened: PhoneOpen) -> (Opened, PhoneSession) {
        match opened {
            PhoneOpen::New { opened, session } => (opened, session),
            PhoneOpen::Existing { .. } => panic!("expected a handshake"),
        }
    }

    fn unwrap_existing(opened: PhoneOpen) -> Opened {
        match opened {
            PhoneOpen::Existing { opened } => opened,
            PhoneOpen::New { .. } => panic!("expected a data message"),
        }
    }

    #[test]
    fn python_handshake_opens_and_a_tampered_or_replayed_body_is_rejected() {
        let pending = generate_desktop_key().unwrap();
        let value = python_phone(&pending);
        let request_id = value["request_id"].as_str().unwrap();
        let mut slot = Some(pending);
        let (opened, mut session) = unwrap_new(
            open_phone_body(value["handshake"].as_str().unwrap(), request_id, &mut slot, None)
                .unwrap(),
        );
        assert_eq!(opened.text, "hello-vector");
        assert!(!opened.join_desktop);
        assert!(slot.is_some(), "a handshake must not burn the desktop key");

        let data = value["data"].as_str().unwrap();
        let second = unwrap_existing(
            open_phone_body(data, request_id, &mut None, Some(&mut session)).unwrap(),
        );
        assert_eq!(second.text, "second-vector");
        assert!(second.join_desktop);
        assert_eq!(
            open_phone_body(data, request_id, &mut None, Some(&mut session)).unwrap_err(),
            SealError::Replay
        );

        // A future counter still has to authenticate. Changing it without
        // the key fails the tag, and the accepted counter stays put.
        let mut tampered = URL_SAFE_NO_PAD.decode(data).unwrap();
        tampered[4] = 2;
        let tampered_b64 = URL_SAFE_NO_PAD.encode(&tampered);
        assert_eq!(
            open_phone_body(&tampered_b64, request_id, &mut None, Some(&mut session)).unwrap_err(),
            SealError::Tamper
        );
        assert_eq!(session.recv, 1, "a failed open must not advance the counter");
    }

    #[test]
    fn swapping_the_phone_public_key_fails_the_handshake() {
        let pending = generate_desktop_key().unwrap();
        let public = URL_SAFE_NO_PAD.encode(pending.public);
        let output = std::process::Command::new("python3")
            .args(["services/relay/phone_crypto.py", "handshake-for", &public])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut raw = URL_SAFE_NO_PAD.decode(value["handshake"].as_str().unwrap()).unwrap();
        raw[3] ^= 0x01;
        let mut slot = Some(pending);
        let error = open_phone_body(
            &URL_SAFE_NO_PAD.encode(&raw),
            "req-vector",
            &mut slot,
            None,
        )
        .unwrap_err();
        assert_eq!(error, SealError::Tamper);
        assert!(slot.is_some(), "a rejected handshake must not burn the desktop key");
    }

    #[test]
    fn reply_seal_opens_in_python_and_the_fragment_is_not_a_query() {
        let pending = generate_desktop_key().unwrap();
        let qr = qr_with_public_key("http://127.0.0.1:8792/", &pending.public);
        assert!(qr.contains("#k="));
        assert!(!qr.split('#').next().unwrap().contains("k="));
        assert_eq!(key_fingerprint(&pending.public).len(), 16);

        let public = URL_SAFE_NO_PAD.encode(pending.public);
        let value = python_phone(&pending);
        let mut slot = Some(pending);
        let (_opened, mut session) = unwrap_new(
            open_phone_body(value["handshake"].as_str().unwrap(), "req-vector", &mut slot, None)
                .unwrap(),
        );
        let sealed = seal_reply(&mut session, "mock-reply", None, "req-vector").unwrap();
        let opened = std::process::Command::new("python3")
            .args([
                "services/relay/phone_crypto.py",
                "open-reply",
                "--desktop-pub",
                &public,
                "--phone-priv",
                value["phone_priv"].as_str().unwrap(),
                "--request-id",
                "req-vector",
                "--body",
                &sealed,
            ])
            .output()
            .unwrap();
        assert!(opened.status.success(), "{}", String::from_utf8_lossy(&opened.stderr));
        let body: Value = serde_json::from_slice(&opened.stdout).unwrap();
        assert_eq!(body["text"], "mock-reply");
        let record = session.to_record();
        let loaded = PhoneSession::from_record(&record).unwrap();
        assert_eq!(loaded.send, session.send);
        assert_eq!(loaded.recv, session.recv);
    }

    #[test]
    fn a_data_counter_of_zero_is_rejected() {
        let pending = generate_desktop_key().unwrap();
        let value = python_phone(&pending);
        let request_id = value["request_id"].as_str().unwrap();
        let mut slot = Some(pending);
        let (_opened, mut session) = unwrap_new(
            open_phone_body(value["handshake"].as_str().unwrap(), request_id, &mut slot, None)
                .unwrap(),
        );
        let mut raw = seal_data(&session.key, DIR_PHONE, 1, &request_id, "x", false, None).unwrap();
        raw[4..12].fill(0);
        assert_eq!(
            open_phone_body(
                &URL_SAFE_NO_PAD.encode(&raw),
                &request_id,
                &mut None,
                Some(&mut session)
            )
            .unwrap_err(),
            SealError::Replay
        );
    }
}
