//! Secure Enclave signing key for personal sign-off.
//!
//! The key is created with `SecAccessControl` `biometryCurrentSet` and
//! `privateKeyUsage`. If that access control cannot be created, the key is
//! created with `userPresence` instead. `LAContext` supplies the prompt's
//! reason text. Signing still goes through `SecKeyCreateSignature`, which is
//! what asks the enclave to use the key.
//!
//! An unsigned dev build that cannot create the key returns `unavailable` so
//! the gate can follow the configured fallback. Cancelling the prompt is a
//! refusal, not a fallback.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{
    kLAPolicyDeviceOwnerAuthentication, kLAPolicyDeviceOwnerAuthenticationWithBiometrics, LAContext,
    LAPolicy,
};
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::item::{ItemClass, ItemSearchOptions, KeyClass, Reference, SearchResult};
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};
use security_framework::passwords_options::AccessControlOptions;

const KEY_LABEL: &str = "plexi.personal-signoff";

pub struct EnclaveError {
    pub unavailable: bool,
    pub message: String,
}

pub fn sign_with_enclave(message: &[u8], reason: &str) -> Result<Vec<u8>, EnclaveError> {
    prompt_for_reason(reason)?;
    let key = ensure_private_key()?;
    key.create_signature(Algorithm::ECDSASignatureMessageX962SHA256, message)
        .map_err(|error| EnclaveError {
            unavailable: false,
            message: format!("Secure Enclave signature failed (Touch ID): {error}"),
        })
}

pub fn verify_with_enclave(message: &[u8], signature: &[u8]) -> bool {
    let Some(private_key) = find_private_key() else {
        return false;
    };
    let Some(public_key) = private_key.public_key() else {
        log::error!("personal_signoff: Secure Enclave key has no public half");
        return false;
    };
    match public_key.verify_signature(
        Algorithm::ECDSASignatureMessageX962SHA256,
        message,
        signature,
    ) {
        Ok(valid) => valid,
        Err(error) => {
            log::info!("personal_signoff: Secure Enclave verify failed: {error}");
            false
        }
    }
}

/// OS password / user-presence dialog. Labeled by the caller as not Touch ID
/// when this is the fallback rather than the enclave path.
pub fn prompt_user_presence(reason: &str) -> Result<(), String> {
    let context = unsafe { LAContext::new() };
    let text = NSString::from_str(reason);
    unsafe { context.setLocalizedReason(&text) };
    let policy = LAPolicy(kLAPolicyDeviceOwnerAuthentication as isize);
    evaluate(&context, policy, &text).map_err(|error| error.message)
}

fn prompt_for_reason(reason: &str) -> Result<(), EnclaveError> {
    let context = unsafe { LAContext::new() };
    let text = NSString::from_str(reason);
    unsafe { context.setLocalizedReason(&text) };
    let biometry = LAPolicy(kLAPolicyDeviceOwnerAuthenticationWithBiometrics as isize);
    let presence = LAPolicy(kLAPolicyDeviceOwnerAuthentication as isize);
    let policy = if unsafe { context.canEvaluatePolicy_error(biometry).is_ok() } {
        log::info!("personal_signoff: LAContext biometryCurrentSet prompt: {reason}");
        biometry
    } else {
        log::info!(
            "personal_signoff: biometry unavailable; LAContext userPresence prompt: {reason}"
        );
        presence
    };
    evaluate(&context, policy, &text)
}

fn evaluate(context: &LAContext, policy: LAPolicy, reason: &NSString) -> Result<(), EnclaveError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let reply = RcBlock::new(move |success: Bool, _error: *mut NSError| {
        let _ = tx.send(success.as_bool());
    });
    unsafe {
        context.evaluatePolicy_localizedReason_reply(policy, reason, &reply);
    }
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(EnclaveError {
            unavailable: false,
            message: "Touch ID or user presence was refused".to_string(),
        }),
        Err(_) => Err(EnclaveError {
            unavailable: false,
            message: "Touch ID prompt timed out".to_string(),
        }),
    }
}

fn ensure_private_key() -> Result<SecKey, EnclaveError> {
    if let Some(key) = find_private_key() {
        return Ok(key);
    }
    match create_private_key(true) {
        Ok(key) => {
            log::info!(
                "personal_signoff: created Secure Enclave key label={KEY_LABEL} access=biometryCurrentSet"
            );
            Ok(key)
        }
        Err(error) => {
            log::info!(
                "personal_signoff: biometryCurrentSet key failed ({}); trying userPresence",
                error.message
            );
            match create_private_key(false) {
                Ok(key) => {
                    log::info!(
                        "personal_signoff: created Secure Enclave key label={KEY_LABEL} access=userPresence"
                    );
                    Ok(key)
                }
                Err(error) => Err(EnclaveError {
                    unavailable: true,
                    message: format!(
                        "Secure Enclave key unavailable (unsigned or unsupported build): {}",
                        error.message
                    ),
                }),
            }
        }
    }
}

fn create_private_key(biometry_current_set: bool) -> Result<SecKey, EnclaveError> {
    let flags = if biometry_current_set {
        AccessControlOptions::BIOMETRY_CURRENT_SET | AccessControlOptions::PRIVATE_KEY_USAGE
    } else {
        AccessControlOptions::USER_PRESENCE | AccessControlOptions::PRIVATE_KEY_USAGE
    };
    let access = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        flags.bits(),
    )
    .map_err(|error| EnclaveError {
        unavailable: true,
        message: format!("SecAccessControl failed: {error}"),
    })?;
    let mut options = GenerateKeyOptions::default();
    options
        .set_key_type(KeyType::ec_sec_prime_random())
        .set_size_in_bits(256)
        .set_token(Token::SecureEnclave)
        .set_label(KEY_LABEL)
        .set_location(security_framework::item::Location::DataProtectionKeychain)
        .set_access_control(access);
    SecKey::new(&options).map_err(|error| EnclaveError {
        unavailable: true,
        message: format!("SecKeyCreateRandomKey failed: {error}"),
    })
}

fn find_private_key() -> Option<SecKey> {
    let results = ItemSearchOptions::new()
        .class(ItemClass::key())
        .key_class(KeyClass::private())
        .label(KEY_LABEL)
        .load_refs(true)
        .search()
        .ok()?;
    for result in results {
        if let SearchResult::Ref(Reference::Key(key)) = result {
            return Some(key);
        }
    }
    None
}
