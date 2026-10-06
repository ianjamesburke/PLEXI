//! Personal sign-off. A click, a posture allow-list, and an unsigned grant
//! cannot satisfy a tool marked `requires: personal_signoff`.
//!
//! Three tiers:
//! - click — the existing approval sheet
//! - `each_time` — a fresh signature on every execution
//! - `time_boxed` — one signature mints a revocable grant (default 7 days)
//!
//! macOS signs with a Secure Enclave key (`biometryCurrentSet`, falling back
//! to `userPresence`). Other builds, and unsigned dev builds where the
//! enclave is unavailable, follow `[permissions.personal_signoff] fallback`:
//! `password` (labeled OS password, not Touch ID) or `refuse`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::{ActorScope, ActorType, ResourceScope};

pub const TIME_BOXED_TTL_SECS: i64 = 7 * 24 * 60 * 60;
const CHALLENGE_TTL_SECS: i64 = 120;
const MIN_TTL_SECS: i64 = 60;
const MAX_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// Elevated tier. Click is the absence of a requirement, not a value here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignoffTier {
    EachTime,
    TimeBoxed,
}

impl SignoffTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EachTime => "each_time",
            Self::TimeBoxed => "time_boxed",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "time_boxed" => Self::TimeBoxed,
            _ => Self::EachTime,
        }
    }

    /// Each-time is stricter. A looser marker cannot replace it.
    pub fn stricter(self, other: Self) -> Self {
        match (self, other) {
            (Self::EachTime, _) | (_, Self::EachTime) => Self::EachTime,
            _ => Self::TimeBoxed,
        }
    }
}

/// What a manifest or grant marker is asking for. Unknown sign-off values
/// stay on each-time so they cannot fall through to a click.
pub fn tier_from_manifest(requires: Option<&str>, signoff: Option<&str>) -> Option<SignoffTier> {
    if requires != Some("personal_signoff") {
        return None;
    }
    Some(match signoff {
        Some("time_boxed") => SignoffTier::TimeBoxed,
        Some("each_time") | None => SignoffTier::EachTime,
        Some(other) => {
            log::error!(
                "personal_signoff: unknown signoff '{other}' on a personal_signoff tool; using each_time"
            );
            SignoffTier::EachTime
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignoffFallback {
    Password,
    Refuse,
}

impl SignoffFallback {
    fn parse(raw: &str) -> Self {
        match raw {
            "password" => Self::Password,
            "refuse" => Self::Refuse,
            other => {
                log::error!(
                    "personal_signoff: unknown fallback '{other}'; refusing elevated calls"
                );
                Self::Refuse
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Refuse => "refuse",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SignoffSettings {
    pub fallback: SignoffFallback,
    pub time_boxed_ttl_secs: i64,
}

impl Default for SignoffSettings {
    fn default() -> Self {
        Self {
            fallback: SignoffFallback::Refuse,
            time_boxed_ttl_secs: TIME_BOXED_TTL_SECS,
        }
    }
}

impl SignoffSettings {
    /// Read `[permissions.personal_signoff]` from the profile config.
    /// A missing table refuses. An unknown fallback refuses.
    pub fn load(profile_dir: &Path) -> Self {
        let path = profile_dir.join("config.toml");
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::error!(
                    "personal_signoff: failed to read {}: {error}; refusing",
                    path.display()
                );
                return Self::default();
            }
        };
        let config: crate::config::PlexiConfig = match toml::from_str(&raw) {
            Ok(config) => config,
            Err(error) => {
                log::error!(
                    "personal_signoff: failed to parse {}: {error}; refusing",
                    path.display()
                );
                return Self::default();
            }
        };
        let Some(table) = config
            .permissions
            .as_ref()
            .and_then(|permissions| permissions.personal_signoff.as_ref())
        else {
            return Self::default();
        };
        let fallback = table
            .fallback
            .as_deref()
            .map(SignoffFallback::parse)
            .unwrap_or(SignoffFallback::Refuse);
        let ttl = table
            .time_boxed_ttl_secs
            .map(|secs| secs.clamp(MIN_TTL_SECS, MAX_TTL_SECS))
            .unwrap_or(TIME_BOXED_TTL_SECS);
        log::info!(
            "personal_signoff: fallback={} time_boxed_ttl_secs={ttl}",
            fallback.as_str()
        );
        Self {
            fallback,
            time_boxed_ttl_secs: ttl,
        }
    }
}

/// Fields the signature covers. Rebuilt from the grant on every use so an
/// edited expiry, actor, or argument hash no longer verifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignoffParts {
    pub actor_type: ActorType,
    pub actor_scope: ActorScope,
    pub trust_origin: String,
    pub actor_id: String,
    pub resource_scope: ResourceScope,
    pub resource_id: Option<String>,
    pub action: String,
    pub args_fingerprint: String,
    pub nonce: String,
    pub expiry: i64,
    pub deadline: i64,
    pub package: String,
    pub tier: SignoffTier,
}

pub fn canonical_message(parts: &SignoffParts) -> String {
    format!(
        "plexi-signoff-v1\nactor={}:{}:{}:{}\nresource={}:{}\naction={}\nargs={}\nnonce={}\nexpiry={}\ndeadline={}\npackage={}\ntier={}\n",
        actor_type_name(parts.actor_type),
        actor_scope_name(parts.actor_scope),
        parts.trust_origin,
        parts.actor_id,
        resource_scope_name(parts.resource_scope),
        parts.resource_id.as_deref().unwrap_or("-"),
        parts.action,
        parts.args_fingerprint,
        parts.nonce,
        parts.expiry,
        parts.deadline,
        parts.package,
        parts.tier.as_str(),
    )
}

fn actor_type_name(actor: ActorType) -> &'static str {
    match actor {
        ActorType::App => "app",
        ActorType::Agent => "agent",
        ActorType::System => "system",
        ActorType::ManagedPolicy => "managed_policy",
    }
}

fn actor_scope_name(scope: ActorScope) -> &'static str {
    match scope {
        ActorScope::BuiltIn => "built_in",
        ActorScope::User => "user",
        ActorScope::Workspace => "workspace",
        ActorScope::Marketplace => "marketplace",
        ActorScope::Managed => "managed",
    }
}

fn resource_scope_name(scope: ResourceScope) -> &'static str {
    match scope {
        ResourceScope::Workspace => "workspace",
        ResourceScope::Pane => "pane",
        ResourceScope::Document => "document",
        ResourceScope::Game => "game",
        ResourceScope::Path => "path",
        ResourceScope::PackageIdentity => "package_identity",
        ResourceScope::Account => "account",
        ResourceScope::Global => "global",
    }
}

pub trait PersonalSigner: Send + Sync {
    fn mechanism_name(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn sign(&self, message: &[u8], reason: &str) -> Result<Vec<u8>, String>;
    fn verify(&self, message: &[u8], signature: &[u8]) -> bool;
    fn verify_mechanism(&self, mechanism: &str, message: &[u8], signature: &[u8]) -> bool {
        mechanism == self.mechanism_name() && self.verify(message, signature)
    }
}

/// In-memory signer for gate tests. Not Touch ID.
#[cfg(test)]
pub struct MockSigner {
    key: Vec<u8>,
}

#[cfg(test)]
impl MockSigner {
    pub fn new() -> Self {
        Self {
            key: b"plexi-test-signoff-key".to_vec(),
        }
    }

    fn mac(&self, message: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(b"plexi-mock-signoff-v1");
        hasher.update(&self.key);
        hasher.update(message);
        hasher.finalize().to_vec()
    }
}

#[cfg(test)]
impl Default for MockSigner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl PersonalSigner for MockSigner {
    fn mechanism_name(&self) -> &'static str {
        "mock"
    }

    fn label(&self) -> &'static str {
        "mock signer (not Touch ID)"
    }

    fn sign(&self, message: &[u8], reason: &str) -> Result<Vec<u8>, String> {
        log::info!(
            "personal_signoff: mock signer (not Touch ID) reason={reason}"
        );
        Ok(self.mac(message))
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        self.mac(message) == signature
    }
}

/// Always refuses. The default when the Secure Enclave is unavailable and
/// fallback is `refuse`.
pub struct RefuseSigner;

impl PersonalSigner for RefuseSigner {
    fn mechanism_name(&self) -> &'static str {
        "refuse"
    }

    fn label(&self) -> &'static str {
        "refused (not Touch ID)"
    }

    fn sign(&self, _message: &[u8], reason: &str) -> Result<Vec<u8>, String> {
        log::info!("personal_signoff: refused (not Touch ID) reason={reason}");
        Err("personal sign-off refused on this build (not Touch ID)".to_string())
    }

    fn verify(&self, _message: &[u8], _signature: &[u8]) -> bool {
        false
    }
}

type PasswordPrompt = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// OS password fallback. The prompt and the audit label say this is not Touch ID.
pub struct OsPasswordSigner {
    key: Vec<u8>,
    prompt: PasswordPrompt,
}

impl OsPasswordSigner {
    pub fn open(profile_dir: &Path) -> Self {
        Self {
            key: load_or_create_dev_key(profile_dir),
            prompt: Box::new(prompt_os_password),
        }
    }

    #[cfg(test)]
    pub fn for_test(
        prompt: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            key: b"test-os-password-key".to_vec(),
            prompt: Box::new(prompt),
        }
    }

    fn mac(&self, message: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(b"plexi-os-password-v1");
        hasher.update(&self.key);
        hasher.update(message);
        hasher.finalize().to_vec()
    }
}

impl PersonalSigner for OsPasswordSigner {
    fn mechanism_name(&self) -> &'static str {
        "os_password"
    }

    fn label(&self) -> &'static str {
        "OS password (not Touch ID)"
    }

    fn sign(&self, message: &[u8], reason: &str) -> Result<Vec<u8>, String> {
        log::info!("personal_signoff: OS password prompt (not Touch ID) reason={reason}");
        (self.prompt)(reason)?;
        Ok(self.mac(message))
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        self.mac(message) == signature
    }
}

fn load_or_create_dev_key(profile_dir: &Path) -> Vec<u8> {
    let path = profile_dir.join("personal-signoff-dev.key");
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            return bytes;
        }
        log::error!(
            "personal_signoff: {} is not a 32-byte dev key; replacing it",
            path.display()
        );
    }
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    if let Err(error) = std::fs::create_dir_all(profile_dir) {
        log::error!(
            "personal_signoff: failed to create {}: {error}",
            profile_dir.display()
        );
        return key.to_vec();
    }
    if let Err(error) = std::fs::write(&path, key) {
        log::error!("personal_signoff: failed to write {}: {error}", path.display());
        return key.to_vec();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            log::error!(
                "personal_signoff: failed to restrict {}: {error}",
                path.display()
            );
        }
    }
    log::info!(
        "personal_signoff: wrote OS password signer key at {} (not a Secure Enclave key)",
        path.display()
    );
    key.to_vec()
}

fn prompt_os_password(reason: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::enclave_signoff::prompt_user_presence(reason)
    }
    #[cfg(not(target_os = "macos"))]
    {
        prompt_sudo(reason)
    }
}

#[cfg(not(target_os = "macos"))]
fn prompt_sudo(reason: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    eprint!("OS password (not Touch ID) — {reason}: ");
    let password = read_hidden_tty()?;
    let mut child = Command::new("sudo")
        .args(["-S", "-k", "-p", "", "-v"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("OS password prompt (not Touch ID) could not run sudo: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let status = child
        .wait()
        .map_err(|error| format!("OS password prompt (not Touch ID) sudo failed: {error}"))?;
    if let Some(stderr) = child.stderr.take() {
        let mut lines = BufReader::new(stderr).lines();
        if let Some(Ok(line)) = lines.next() {
            if !status.success() {
                log::info!("personal_signoff: sudo rejected the OS password: {line}");
            }
        }
    }
    if status.success() {
        log::info!("personal_signoff: OS password accepted (not Touch ID)");
        Ok(())
    } else {
        Err("OS password was refused (not Touch ID)".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn read_hidden_tty() -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            format!("OS password prompt (not Touch ID) needs a terminal: {error}")
        })?;
    let fd = std::os::fd::AsRawFd::as_raw_fd(&file);
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        return Err("OS password prompt (not Touch ID) could not read terminal attributes".into());
    }
    let mut hidden = original;
    hidden.c_lflag &= !libc::ECHO;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
        return Err("OS password prompt (not Touch ID) could not hide terminal echo".into());
    }
    let mut reader = BufReader::new(file.try_clone().map_err(|error| error.to_string())?);
    let mut line = String::new();
    let read = reader.read_line(&mut line);
    unsafe {
        libc::tcsetattr(fd, libc::TCSANOW, &original);
    }
    let mut tty = file;
    let _ = tty.write_all(b"\n");
    read.map_err(|error| format!("OS password prompt (not Touch ID) read failed: {error}"))?;
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    if line.is_empty() {
        return Err("OS password prompt (not Touch ID) received an empty password".into());
    }
    Ok(line)
}

enum EnclaveAttempt {
    #[cfg(target_os = "macos")]
    Signature(Vec<u8>),
    /// The person cancelled. Do not fall through to a password.
    #[cfg(target_os = "macos")]
    Refused(String),
    /// No enclave key on this build. Fall back to the configured policy.
    Unavailable(String),
}

/// Host signer. macOS tries the Secure Enclave first. Cancellation is a
/// refusal. An unavailable enclave uses the configured fallback.
pub struct ProductionSigner {
    fallback: SignoffFallback,
    password: OsPasswordSigner,
    enclave_ready: Mutex<bool>,
    enclave_unavailable: Mutex<bool>,
}

impl ProductionSigner {
    pub fn new(profile_dir: &Path, settings: SignoffSettings) -> Self {
        log::info!(
            "personal_signoff: production signer fallback={} (Secure Enclave is tried first on macOS)",
            settings.fallback.as_str()
        );
        Self {
            fallback: settings.fallback,
            password: OsPasswordSigner::open(profile_dir),
            enclave_ready: Mutex::new(false),
            enclave_unavailable: Mutex::new(false),
        }
    }

    fn try_enclave(&self, message: &[u8], reason: &str) -> EnclaveAttempt {
        #[cfg(target_os = "macos")]
        {
            match crate::platform::enclave_signoff::sign_with_enclave(message, reason) {
                Ok(signature) => {
                    *self.enclave_ready.lock().unwrap_or_else(|e| e.into_inner()) = true;
                    EnclaveAttempt::Signature(signature)
                }
                Err(error) if error.unavailable => EnclaveAttempt::Unavailable(error.message),
                Err(error) => EnclaveAttempt::Refused(error.message),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (message, reason);
            EnclaveAttempt::Unavailable(
                "Secure Enclave is not available on this build (not Touch ID)".to_string(),
            )
        }
    }
}

impl PersonalSigner for ProductionSigner {
    fn mechanism_name(&self) -> &'static str {
        if *self.enclave_ready.lock().unwrap_or_else(|e| e.into_inner()) {
            return "touch_id";
        }
        let unavailable = *self
            .enclave_unavailable
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // macOS reports Touch ID until the enclave has been shown unavailable.
        // Other builds have no enclave, so the configured fallback is the label.
        #[cfg(target_os = "macos")]
        if !unavailable {
            return "touch_id";
        }
        #[cfg(not(target_os = "macos"))]
        let _ = unavailable;
        match self.fallback {
            SignoffFallback::Password => "os_password",
            SignoffFallback::Refuse => "refuse",
        }
    }

    fn label(&self) -> &'static str {
        match self.mechanism_name() {
            "touch_id" => "Touch ID",
            "os_password" => "OS password (not Touch ID)",
            _ => "refused (not Touch ID)",
        }
    }

    fn sign(&self, message: &[u8], reason: &str) -> Result<Vec<u8>, String> {
        match self.try_enclave(message, reason) {
            #[cfg(target_os = "macos")]
            EnclaveAttempt::Signature(signature) => {
                log::info!("personal_signoff: Secure Enclave signed ({})", "Touch ID");
                Ok(signature)
            }
            #[cfg(target_os = "macos")]
            EnclaveAttempt::Refused(detail) => {
                log::info!("personal_signoff: Touch ID refused: {detail}");
                Err(detail)
            }
            EnclaveAttempt::Unavailable(detail) => {
                *self
                    .enclave_unavailable
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = true;
                log::info!(
                    "personal_signoff: {detail}; fallback {}",
                    self.fallback.as_str()
                );
                match self.fallback {
                    SignoffFallback::Password => {
                        let reason = format!("{reason} — OS password (not Touch ID)");
                        self.password.sign(message, &reason)
                    }
                    SignoffFallback::Refuse => Err(format!(
                        "{detail}; personal sign-off refused (not Touch ID)"
                    )),
                }
            }
        }
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        self.verify_mechanism("touch_id", message, signature)
            || self.verify_mechanism("os_password", message, signature)
    }

    fn verify_mechanism(&self, mechanism: &str, message: &[u8], signature: &[u8]) -> bool {
        match mechanism {
            "touch_id" => {
                #[cfg(target_os = "macos")]
                {
                    crate::platform::enclave_signoff::verify_with_enclave(message, signature)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (message, signature);
                    false
                }
            }
            "os_password" => self.password.verify(message, signature),
            _ => false,
        }
    }
}

pub fn production_signer(profile_dir: &Path, settings: SignoffSettings) -> Arc<dyn PersonalSigner> {
    Arc::new(ProductionSigner::new(profile_dir, settings))
}

pub fn challenge_ttl_secs() -> i64 {
    CHALLENGE_TTL_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_message_binds_actor_resource_action_args_nonce_and_expiry() {
        let parts = SignoffParts {
            actor_type: ActorType::Agent,
            actor_scope: ActorScope::User,
            trust_origin: "host".to_string(),
            actor_id: "agent:chess".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: Some("game-1".to_string()),
            action: "chess.play".to_string(),
            args_fingerprint: "abc".to_string(),
            nonce: "nonce-1".to_string(),
            expiry: 1_700_000_000,
            deadline: 1_700_000_120,
            package: "chess".to_string(),
            tier: SignoffTier::EachTime,
        };
        let message = canonical_message(&parts);
        assert!(message.starts_with("plexi-signoff-v1\n"));
        assert!(message.contains("actor=agent:user:host:agent:chess\n"));
        assert!(message.contains("resource=game:game-1\n"));
        assert!(message.contains("action=chess.play\n"));
        assert!(message.contains("args=abc\n"));
        assert!(message.contains("nonce=nonce-1\n"));
        assert!(message.contains("expiry=1700000000\n"));
        assert!(message.contains("deadline=1700000120\n"));
        assert!(message.contains("tier=each_time\n"));
        let mut other = parts.clone();
        other.args_fingerprint = "def".to_string();
        assert_ne!(message, canonical_message(&other));
    }

    #[test]
    fn unknown_fallback_and_missing_config_refuse() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            SignoffSettings::load(dir.path()).fallback,
            SignoffFallback::Refuse
        );
        std::fs::write(
            dir.path().join("config.toml"),
            "[permissions.personal_signoff]\nfallback = \"click\"\ntime_boxed_ttl_secs = 10\n",
        )
        .unwrap();
        let settings = SignoffSettings::load(dir.path());
        assert_eq!(settings.fallback, SignoffFallback::Refuse);
        assert_eq!(settings.time_boxed_ttl_secs, MIN_TTL_SECS);
        std::fs::write(
            dir.path().join("config.toml"),
            "[permissions.personal_signoff]\nfallback = \"password\"\ntime_boxed_ttl_secs = 604800\n",
        )
        .unwrap();
        let settings = SignoffSettings::load(dir.path());
        assert_eq!(settings.fallback, SignoffFallback::Password);
        assert_eq!(settings.time_boxed_ttl_secs, TIME_BOXED_TTL_SECS);
    }

    #[test]
    fn manifest_each_time_is_not_downgraded_by_an_unknown_signoff_value() {
        assert_eq!(
            tier_from_manifest(Some("personal_signoff"), Some("click")),
            Some(SignoffTier::EachTime)
        );
        assert_eq!(
            tier_from_manifest(Some("personal_signoff"), Some("time_boxed")),
            Some(SignoffTier::TimeBoxed)
        );
        assert_eq!(tier_from_manifest(None, Some("time_boxed")), None);
        assert_eq!(
            SignoffTier::EachTime.stricter(SignoffTier::TimeBoxed),
            SignoffTier::EachTime
        );
    }

    #[test]
    fn os_password_signer_is_labeled_and_is_not_touch_id() {
        let signer = OsPasswordSigner::for_test(|_| Ok(()));
        assert_eq!(signer.label(), "OS password (not Touch ID)");
        assert_eq!(signer.mechanism_name(), "os_password");
        assert_ne!(signer.mechanism_name(), "touch_id");
        let signature = signer.sign(b"message", "approve chess.play").unwrap();
        assert!(signer.verify(b"message", &signature));
        assert!(!signer.verify_mechanism("touch_id", b"message", &signature));
        assert!(signer.verify_mechanism("os_password", b"message", &signature));
        let refused = OsPasswordSigner::for_test(|_| Err("no".into()));
        assert!(refused.sign(b"message", "approve").is_err());
    }

    #[test]
    fn mock_signature_does_not_verify_as_touch_id() {
        let signer = MockSigner::new();
        let signature = signer.sign(b"message", "test").unwrap();
        assert!(signer.verify_mechanism("mock", b"message", &signature));
        assert!(!signer.verify_mechanism("touch_id", b"message", &signature));
        assert_eq!(signer.label(), "mock signer (not Touch ID)");
    }
}
