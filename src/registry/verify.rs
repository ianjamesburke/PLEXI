use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::path::Path;

pub const PLEXI_TEAM_PUBLIC_KEY: [u8; 32] = [0; 32];

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("could not read bundle {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid Ed25519 public key")]
    PublicKey,
    #[error("invalid Ed25519 signature")]
    Signature,
    #[error("bundle signature verification failed")]
    Failed,
}

pub fn verify_bundle(bundle_path: &Path, sig: &[u8], pubkey: &[u8]) -> Result<(), VerifyError> {
    let bytes = std::fs::read(bundle_path).map_err(|source| VerifyError::Read {
        path: bundle_path.display().to_string(),
        source,
    })?;
    verify_bytes(&bytes, sig, pubkey)
}

pub fn verify_bytes(bytes: &[u8], sig: &[u8], pubkey: &[u8]) -> Result<(), VerifyError> {
    let key_bytes: [u8; 32] = pubkey.try_into().map_err(|_| VerifyError::PublicKey)?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| VerifyError::PublicKey)?;
    let sig_bytes: [u8; 64] = sig.try_into().map_err(|_| VerifyError::Signature)?;
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key
        .verify(bytes, &signature)
        .map_err(|_| VerifyError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn signature_verification_passes_and_fails_closed() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let message = b"bundle";
        let sig = signing.sign(message);
        let pubkey = signing.verifying_key().to_bytes();
        verify_bytes(message, &sig.to_bytes(), &pubkey).unwrap();
        let err = verify_bytes(b"tampered", &sig.to_bytes(), &pubkey).unwrap_err();
        assert!(matches!(err, VerifyError::Failed));
    }
}
