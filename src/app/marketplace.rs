//! Hosted marketplace layer — registry catalog, publisher submission, and the
//! paid-app license + payment boundary (stint `0018`-`0021`).
//!
//! # Invariant carried from `docs/prm/marketplace-hosted.md`
//!
//! Hosted services **distribute metadata and broker money**. They never become
//! a dependency for running a local app. Everything in this module is a
//! discovery / purchase surface layered *on top of* the local package format
//! (`crate::app::package`) and the local install flow. An installed app never
//! phones home; the registry being down never breaks an installed app.
//!
//! # What lives here
//!
//! - [`Visibility`] / [`Price`] — the marketplace facts an app declares in its
//!   manifest `[marketplace]` section. They drive public/private listing and
//!   free/paid gating.
//! - [`RegistryEntry`] / [`RegistryIndex`] / [`RegistryClient`] — the hosted
//!   catalog. The index *republishes* what the local validator already produced
//!   (`crate::app::package::PackageReport`); it does not invent a second schema.
//! - [`License`] / [`LicenseStore`] — proof-of-ownership for a paid app, stored
//!   on disk per-channel. Real and tested. A license is the only thing the host
//!   checks before installing a paid app.
//! - [`PaymentProvider`] / [`StubPaymentProvider`] / [`payment_provider`] — the
//!   payment boundary. Today the factory returns the stub, which fails closed
//!   with [`PaymentError::NotConfigured`]. A real provider (Stripe, Polar, …)
//!   slots into [`payment_provider`] with no other change anywhere — the license
//!   store, the install gate, and the CLI all consume the trait, never a
//!   concrete provider.
//! - [`PublishClient`] / [`Submission`] — the publisher submission flow. Builds
//!   a validated submission from a local package and posts it to a configured
//!   endpoint; fails closed and honestly when no endpoint is set.

use crate::app::package::PackageReport;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Schema version for the registry index and license files. Bumped whenever a
/// required field changes so an older host fails loud rather than silently
/// mis-reading a newer catalog.
pub const MARKETPLACE_SCHEMA_VERSION: u32 = 1;

/// Default hosted registry index URL. Overridable via
/// `[marketplace].registry_url` in config. Honest default: points at the
/// product domain (`plexiapp.com`), never a placeholder.
pub const DEFAULT_REGISTRY_URL: &str = "https://plexiapp.com/registry/v1/index.json";

/// Default CDN base for package artifacts, addressed by checksum. Overridable
/// via `[marketplace].cdn_url`.
pub const DEFAULT_CDN_URL: &str = "https://plexiapp.com/registry/v1/packages";

// ── Visibility ────────────────────────────────────────────────────────────────

/// Who can discover an app. Local-first default is [`Visibility::Private`]: an
/// app is not public until its author explicitly publishes it. Drives whether
/// `app browse`/`app search` list it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Listed in the public catalog and returned by search/browse.
    Public,
    /// Installable by exact id or direct URL, but never listed by browse/search.
    Unlisted,
    /// Not in the hosted catalog at all. The local-only / pre-publish default.
    #[default]
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Unlisted => "unlisted",
            Visibility::Private => "private",
        }
    }

    /// Whether `app browse` (the public catalog view) should list this app.
    pub fn is_listed(self) -> bool {
        matches!(self, Visibility::Public)
    }
}

// ── Price ─────────────────────────────────────────────────────────────────────

/// What an app costs. Money is stored in integer minor units (cents) to avoid
/// float rounding. [`Price::Free`] is the default — an app costs nothing unless
/// it explicitly says otherwise.
///
/// Serialized form in `manifest.toml`:
/// - free: `price = "free"` (or omitted)
/// - paid: `price = { amount_cents = 499, currency = "usd" }`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Price {
    /// A keyword form so `price = "free"` parses cleanly.
    Keyword(FreeKeyword),
    /// A paid app with an explicit amount + ISO-4217 currency.
    Paid {
        amount_cents: u64,
        #[serde(default = "default_currency")]
        currency: String,
    },
}

/// The only legal string value for [`Price::Keyword`]. A dedicated enum keeps
/// `price = "freee"` a loud parse error instead of a silently-misread paid app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FreeKeyword {
    Free,
}

fn default_currency() -> String {
    "usd".to_string()
}

impl Default for Price {
    fn default() -> Self {
        Price::Keyword(FreeKeyword::Free)
    }
}

impl Price {
    pub fn is_free(&self) -> bool {
        matches!(self, Price::Keyword(FreeKeyword::Free))
    }

    /// Human-readable price for the trust/install sheet, e.g. `"Free"` or
    /// `"$4.99 USD"`.
    pub fn display(&self) -> String {
        match self {
            Price::Keyword(FreeKeyword::Free) => "Free".to_string(),
            Price::Paid {
                amount_cents,
                currency,
            } => {
                let major = *amount_cents / 100;
                let minor = *amount_cents % 100;
                let sym = if currency.eq_ignore_ascii_case("usd") {
                    "$"
                } else {
                    ""
                };
                format!("{sym}{major}.{minor:02} {}", currency.to_uppercase())
            }
        }
    }
}

// ── Manifest section ──────────────────────────────────────────────────────────

/// The optional `[marketplace]` manifest section. Absent entirely for a
/// local-only app, in which case the app is free + private and cannot be
/// published until the author adds this section.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MarketplaceManifest {
    /// Listing visibility. Defaults to [`Visibility::Private`].
    #[serde(default)]
    pub visibility: Visibility,
    /// Price. Defaults to [`Price::Free`].
    #[serde(default)]
    pub price: Price,
    /// Publisher account / org slug. Required to publish; `None` for local apps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
}

// ── Registry catalog ──────────────────────────────────────────────────────────

/// One app in the hosted catalog. Republishes the local validator output
/// ([`PackageReport`]) plus the marketplace facts and the CDN address. The host
/// never trusts a registry-supplied trust label over its own validation — on
/// install it re-validates the downloaded package and recomputes the label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Publisher account/org slug.
    #[serde(default)]
    pub publisher: String,
    /// Capability strings, exactly as the local validator reported them.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Discovery tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Trust label string the publisher's local validator produced. Advisory —
    /// re-derived locally on install. Never claims more than the local check.
    #[serde(default)]
    pub trust_label: String,
    #[serde(default)]
    pub visibility: Visibility,
    #[serde(default)]
    pub price: Price,
    /// sha256 of the `.plexipkg` artifact. The CDN is addressed by this hash, so
    /// a tampered download fails the local checksum check on install.
    pub checksum: String,
    /// Source spec the host can resolve to fetch the artifact when no explicit
    /// `package_url` is given (e.g. `"github:owner/repo"`). Lets install-from-
    /// registry reuse the existing source resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Direct CDN URL for the `.plexipkg`. When absent, the client builds one
    /// from the configured CDN base + checksum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_url: Option<String>,
    /// Declared minimum host version (`[requires].plexi_min`). Lets the catalog
    /// and install gate reject an incompatible app before download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_plexi_min: Option<String>,
    /// Declared soft host-version ceiling (`[requires].plexi_max`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_plexi_max: Option<String>,
}

impl RegistryEntry {
    /// Build a catalog entry from a locally-validated package plus the
    /// marketplace manifest facts. This is the publisher-side path: what
    /// `app publish` would submit, and what the registry republishes verbatim.
    pub fn from_report(
        report: &PackageReport,
        marketplace: &MarketplaceManifest,
        trust_label: &str,
        checksum: String,
        tags: Vec<String>,
    ) -> Self {
        RegistryEntry {
            id: report.id.clone(),
            name: report.name.clone(),
            version: report.version.clone(),
            description: String::new(),
            publisher: marketplace.publisher.clone().unwrap_or_default(),
            capabilities: report.capabilities.iter().map(|c| c.as_str().to_string()).collect(),
            tags,
            trust_label: trust_label.to_string(),
            visibility: marketplace.visibility,
            price: marketplace.price.clone(),
            checksum,
            source: None,
            package_url: None,
            requires_plexi_min: report.requires_plexi_min.clone(),
            requires_plexi_max: report.requires_plexi_max.clone(),
        }
    }
}

/// The whole catalog. Versioned so an older host refuses a newer schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub schema_version: u32,
    #[serde(default)]
    pub apps: Vec<RegistryEntry>,
}

impl RegistryIndex {
    /// Apps that should appear in `app browse` — public listings only.
    pub fn listed(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.apps.iter().filter(|e| e.visibility.is_listed())
    }

    /// Case-insensitive substring search over id, name, description, and tags.
    /// Only searches listed (public) apps — unlisted/private apps are not
    /// discoverable by query, only by exact id.
    pub fn search<'a>(&'a self, query: &str) -> Vec<&'a RegistryEntry> {
        let q = query.to_lowercase();
        self.listed()
            .filter(|e| {
                e.id.to_lowercase().contains(&q)
                    || e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Look up a single entry by exact id, regardless of visibility. Used by
    /// install-from-registry — you can install an unlisted/private app if you
    /// know its id and (for paid apps) hold a license.
    pub fn get(&self, id: &str) -> Option<&RegistryEntry> {
        self.apps.iter().find(|e| e.id == id)
    }
}

/// Why a registry/network operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("registry fetch failed: {0}")]
    Fetch(String),
    #[error("registry response was not valid JSON: {0}")]
    Parse(String),
    #[error("registry schema_version {found} is newer than supported ({supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("no app with id '{0}' in the registry")]
    NotFound(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("downloaded artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
}

/// Thin HTTP client for the hosted catalog + CDN. Holds only URLs; constructing
/// it does no I/O. All methods fail closed and log at `info`/`warn`.
pub struct RegistryClient {
    index_url: String,
    cdn_base: String,
}

impl RegistryClient {
    /// Build a client from the resolved config (or defaults).
    pub fn new(index_url: Option<String>, cdn_base: Option<String>) -> Self {
        RegistryClient {
            index_url: index_url.unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string()),
            cdn_base: cdn_base.unwrap_or_else(|| DEFAULT_CDN_URL.to_string()),
        }
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
    }

    /// Fetch and parse the catalog index.
    pub fn fetch_index(&self) -> Result<RegistryIndex, RegistryError> {
        log::info!("marketplace: fetching registry index from {}", self.index_url);
        let body = Self::agent()
            .get(&self.index_url)
            .call()
            .map_err(|e| RegistryError::Fetch(e.to_string()))?
            .into_string()
            .map_err(|e| RegistryError::Fetch(e.to_string()))?;
        let index: RegistryIndex =
            serde_json::from_str(&body).map_err(|e| RegistryError::Parse(e.to_string()))?;
        if index.schema_version > MARKETPLACE_SCHEMA_VERSION {
            return Err(RegistryError::SchemaTooNew {
                found: index.schema_version,
                supported: MARKETPLACE_SCHEMA_VERSION,
            });
        }
        log::info!("marketplace: index loaded, {} apps", index.apps.len());
        Ok(index)
    }

    /// Fetch the index and return one entry by exact id.
    pub fn fetch_entry(&self, id: &str) -> Result<RegistryEntry, RegistryError> {
        self.fetch_index()?
            .get(id)
            .cloned()
            .ok_or_else(|| RegistryError::NotFound(id.to_string()))
    }

    /// Resolve the artifact URL for an entry: explicit `package_url`, else
    /// `<cdn_base>/<checksum>.plexipkg`.
    pub fn artifact_url(&self, entry: &RegistryEntry) -> String {
        entry
            .package_url
            .clone()
            .unwrap_or_else(|| format!("{}/{}.plexipkg", self.cdn_base, entry.checksum))
    }

    /// Download an entry's `.plexipkg` artifact to `dest`, verifying that the
    /// downloaded bytes match the entry's checksum. A checksum mismatch is a
    /// hard error — the CDN is addressed by hash, so a mismatch means tampering
    /// or corruption. Returns the path written.
    pub fn download_package(
        &self,
        entry: &RegistryEntry,
        dest: &std::path::Path,
    ) -> Result<PathBuf, RegistryError> {
        let url = self.artifact_url(entry);
        log::info!("marketplace: downloading '{}' artifact from {url}", entry.id);
        let resp = Self::agent()
            .get(&url)
            .call()
            .map_err(|e| RegistryError::Download(e.to_string()))?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RegistryError::Download(format!("create {}: {e}", parent.display())))?;
        }
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(dest)
            .map_err(|e| RegistryError::Download(format!("create {}: {e}", dest.display())))?;
        std::io::copy(&mut reader, &mut file)
            .map_err(|e| RegistryError::Download(format!("write {}: {e}", dest.display())))?;
        drop(file);

        let actual = crate::app::package::sha256_file(dest)
            .map_err(|e| RegistryError::Download(format!("checksum {}: {e}", dest.display())))?;
        if actual != entry.checksum {
            let _ = std::fs::remove_file(dest);
            return Err(RegistryError::ChecksumMismatch {
                expected: entry.checksum.clone(),
                actual,
            });
        }
        log::info!("marketplace: downloaded + verified '{}' to {}", entry.id, dest.display());
        Ok(dest.to_path_buf())
    }
}

// ── Licenses ──────────────────────────────────────────────────────────────────

/// A receipt proving a user bought a paid app — like owning a purchased
/// template file. Issued by a [`PaymentProvider`] on a successful purchase and
/// persisted on disk. It is **never checked to run the app**: once the package
/// is installed it runs freely, same path as a free app. The receipt only
/// matters for re-downloading the paid artifact later without paying twice. The
/// `token` is an opaque provider-issued string a future server uses to verify
/// ownership on re-download.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct License {
    pub schema_version: u32,
    pub app_id: String,
    /// Version this license covers, or `"*"` for any version.
    pub version: String,
    /// ISO-8601 UTC issue time.
    pub issued_at: String,
    /// Opaque provider-issued ownership token, used for server-side re-download
    /// verification when real payment is wired.
    pub token: String,
    /// Which provider issued this license (e.g. `"stub"`, `"stripe"`).
    pub provider: String,
}

impl License {
    /// Whether this license authorizes installing the given app id. v1: same id,
    /// and either a wildcard version or an exact match.
    pub fn authorizes(&self, app_id: &str, version: &str) -> bool {
        self.app_id == app_id && (self.version == "*" || self.version == version)
    }
}

/// On-disk license store, one TOML file per app id under
/// `<config_dir>/licenses/<app_id>.toml`. Per-channel by construction because
/// `config_dir()` is per-channel.
pub struct LicenseStore {
    dir: PathBuf,
}

impl LicenseStore {
    /// Open the store for the active channel.
    pub fn open() -> Self {
        LicenseStore {
            dir: crate::config::config_dir().join("licenses"),
        }
    }

    /// Open a store rooted at an explicit directory (tests).
    #[cfg(test)]
    pub fn open_at(dir: PathBuf) -> Self {
        LicenseStore { dir }
    }

    fn path_for(&self, app_id: &str) -> PathBuf {
        self.dir.join(format!("{app_id}.toml"))
    }

    /// Load the license for an app id, if one is stored.
    pub fn load(&self, app_id: &str) -> Option<License> {
        let path = self.path_for(app_id);
        let text = std::fs::read_to_string(&path).ok()?;
        match toml::from_str::<License>(&text) {
            Ok(lic) => Some(lic),
            Err(e) => {
                log::warn!("marketplace: corrupt license at {}: {e}", path.display());
                None
            }
        }
    }

    /// Persist a license, creating the store dir if needed.
    pub fn save(&self, license: &License) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("could not create license dir {}: {e}", self.dir.display()))?;
        let text = toml::to_string_pretty(license)
            .map_err(|e| format!("could not serialize license: {e}"))?;
        let path = self.path_for(&license.app_id);
        std::fs::write(&path, text)
            .map_err(|e| format!("could not write license {}: {e}", path.display()))?;
        log::info!(
            "marketplace: saved license for '{}' v{} (provider {})",
            license.app_id,
            license.version,
            license.provider
        );
        Ok(())
    }

    /// Whether a stored license authorizes installing this app id + version.
    pub fn has_valid(&self, app_id: &str, version: &str) -> bool {
        self.load(app_id)
            .map(|l| l.authorizes(app_id, version))
            .unwrap_or(false)
    }

    /// All stored licenses, sorted by app id.
    pub fn list(&self) -> Vec<License> {
        let mut out = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Ok(text) = std::fs::read_to_string(entry.path()) {
                        if let Ok(lic) = toml::from_str::<License>(&text) {
                            out.push(lic);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        out
    }
}

// ── Payment boundary ──────────────────────────────────────────────────────────

/// Why a purchase could not complete.
#[derive(Debug, thiserror::Error)]
pub enum PaymentError {
    /// No real payment provider is configured in this build. The stub returns
    /// this for every purchase — it is the expected, honest failure until a
    /// provider (Stripe, Polar, …) is wired into [`payment_provider`].
    #[error(
        "paid apps require a configured payment provider — none is set up in this build. \
         Free apps install normally. See https://plexiapp.com/docs/marketplace"
    )]
    NotConfigured,
    /// The user is not signed in to a marketplace account.
    #[error("a marketplace account is required to purchase paid apps")]
    NoAccount,
}

/// The payment boundary. A purchase turns an account + a paid catalog entry into
/// a [`License`]. This is the single seam a real processor implements; nothing
/// else in the codebase references a concrete provider.
///
/// Factory rule (`CLAUDE.md`): no method may panic. The stub returns `Err` /
/// noop for everything it cannot do.
pub trait PaymentProvider: Send + Sync {
    /// Provider name for logs and license records (e.g. `"stub"`, `"stripe"`).
    fn name(&self) -> &'static str;

    /// Whether this provider can actually charge. The stub returns `false`, so
    /// callers can give a clear "paid apps unavailable" message before prompting.
    fn is_configured(&self) -> bool;

    /// Attempt to purchase `entry` for the logged-in `session`, returning a
    /// [`License`] on success. The stub always fails with
    /// [`PaymentError::NotConfigured`].
    fn purchase(
        &self,
        entry: &RegistryEntry,
        session: &crate::app::account::AccountSession,
    ) -> Result<License, PaymentError>;
}

/// The fail-closed default provider. Charges nothing, issues nothing, and never
/// panics. Replacing this is the entire job of adding real payments.
pub struct StubPaymentProvider;

impl PaymentProvider for StubPaymentProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn is_configured(&self) -> bool {
        false
    }

    fn purchase(
        &self,
        entry: &RegistryEntry,
        _session: &crate::app::account::AccountSession,
    ) -> Result<License, PaymentError> {
        log::warn!(
            "marketplace: purchase of paid app '{}' attempted with stub provider — not configured",
            entry.id
        );
        Err(PaymentError::NotConfigured)
    }
}

/// Resolve the active payment provider from config. Today this only ever returns
/// the stub; a real provider drops in here keyed on `payment_backend` with zero
/// changes at any call site.
///
/// ```ignore
/// match backend.as_deref() {
///     Some("stripe") => Box::new(StripeProvider::from_config(cfg)),
///     _ => Box::new(StubPaymentProvider),
/// }
/// ```
pub fn payment_provider() -> Box<dyn PaymentProvider> {
    let backend = crate::config::marketplace_payment_backend();
    match backend.as_deref() {
        // Real providers slot in here. None exist yet, so everything falls
        // through to the stub. This is intentional and must fail closed.
        Some("none") | None => Box::new(StubPaymentProvider),
        Some(other) => {
            log::warn!(
                "marketplace: payment_backend='{other}' is configured but no provider is built — \
                 falling back to stub (paid installs will fail closed)"
            );
            Box::new(StubPaymentProvider)
        }
    }
}

// ── Install license gate ──────────────────────────────────────────────────────

/// The decision for whether a registry install may proceed, based purely on the
/// entry's price and the local license store. Pure function — no network, no
/// disk beyond the store passed in — so it is fully unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseDecision {
    /// Free app, or a paid app the user already owns. Install may proceed.
    Proceed,
    /// Paid app with no local license. The caller must purchase (or fail).
    NeedsPurchase,
}

/// Decide whether installing `entry` is licensed.
pub fn license_gate(entry: &RegistryEntry, store: &LicenseStore) -> LicenseDecision {
    if entry.price.is_free() {
        return LicenseDecision::Proceed;
    }
    if store.has_valid(&entry.id, &entry.version) {
        LicenseDecision::Proceed
    } else {
        LicenseDecision::NeedsPurchase
    }
}

// ── Publisher submission ──────────────────────────────────────────────────────

/// Lifecycle of a publisher submission. Mirrors the states in
/// `marketplace-hosted.md` §3. The host only ever *creates* a submission
/// (`Submitted`); the hosted review system advances the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
    Submitted,
    InReview,
    Approved,
    ChangesRequested,
    Rejected,
}

/// The metadata payload a publisher sends: the validated catalog entry. The
/// server runs the *same* validator (`0015`) on the separately-uploaded package
/// — this struct is the metadata half of that exchange. The artifact path is
/// tracked by the CLI alongside, not in this wire payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    pub schema_version: u32,
    pub entry: RegistryEntry,
}

/// Why a publish/submit failed.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error(
        "no marketplace submission endpoint is configured in this build — the package was \
         validated and prepared but not uploaded. Set [marketplace].submit_url to enable upload. \
         See https://plexiapp.com/docs/marketplace"
    )]
    NotConfigured,
    #[error("submission upload failed: {0}")]
    Upload(String),
}

/// Client for the publisher submission endpoint. Holds the submit URL; absent
/// URL means submission is not configured and `submit` fails closed.
pub struct PublishClient {
    submit_url: Option<String>,
}

impl PublishClient {
    pub fn new(submit_url: Option<String>) -> Self {
        PublishClient { submit_url }
    }

    /// Upload a prepared submission. Fails closed with
    /// [`PublishError::NotConfigured`] when no endpoint is set — the caller is
    /// expected to have already validated, packaged, and reported what *would*
    /// be submitted, so this is an honest "last mile not wired" boundary rather
    /// than a silent success.
    pub fn submit(&self, submission: &Submission) -> Result<SubmissionState, PublishError> {
        let Some(url) = self.submit_url.as_deref().filter(|u| !u.is_empty()) else {
            return Err(PublishError::NotConfigured);
        };
        log::info!(
            "marketplace: submitting '{}' v{} to {url}",
            submission.entry.id,
            submission.entry.version
        );
        let body = serde_json::to_string(submission)
            .map_err(|e| PublishError::Upload(format!("serialize submission: {e}")))?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build();
        let resp = agent
            .post(url)
            .set("content-type", "application/json")
            .send_string(&body)
            .map_err(|e| PublishError::Upload(e.to_string()))?;
        let text = resp
            .into_string()
            .map_err(|e| PublishError::Upload(e.to_string()))?;
        log::info!("marketplace: submission accepted: {text}");
        Ok(SubmissionState::Submitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, price: Price, vis: Visibility) -> RegistryEntry {
        RegistryEntry {
            id: id.to_string(),
            name: id.to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            publisher: "acme".to_string(),
            capabilities: vec![],
            tags: vec![],
            trust_label: "Unreviewed Python process".to_string(),
            visibility: vis,
            price,
            checksum: "abc123".to_string(),
            source: None,
            package_url: None,
            requires_plexi_min: None,
            requires_plexi_max: None,
        }
    }

    fn paid() -> Price {
        Price::Paid {
            amount_cents: 499,
            currency: "usd".to_string(),
        }
    }

    #[test]
    fn visibility_default_is_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
        assert!(!Visibility::Private.is_listed());
        assert!(!Visibility::Unlisted.is_listed());
        assert!(Visibility::Public.is_listed());
    }

    #[test]
    fn price_default_is_free() {
        assert!(Price::default().is_free());
        assert!(!paid().is_free());
        assert_eq!(paid().display(), "$4.99 USD");
        assert_eq!(Price::default().display(), "Free");
    }

    #[test]
    fn price_parses_free_keyword_and_paid_table() {
        #[derive(Deserialize)]
        struct Holder {
            price: Price,
        }
        let free: Holder = toml::from_str(r#"price = "free""#).unwrap();
        assert!(free.price.is_free());
        let paid: Holder =
            toml::from_str("price = { amount_cents = 1500, currency = \"eur\" }").unwrap();
        assert_eq!(
            paid.price,
            Price::Paid {
                amount_cents: 1500,
                currency: "eur".to_string()
            }
        );
    }

    #[test]
    fn bad_price_keyword_is_loud_error() {
        #[derive(Deserialize)]
        struct Holder {
            #[allow(dead_code)]
            price: Price,
        }
        assert!(toml::from_str::<Holder>(r#"price = "freee""#).is_err());
    }

    #[test]
    fn index_search_only_lists_public_apps() {
        let index = RegistryIndex {
            schema_version: 1,
            apps: vec![
                entry("public-notes", Price::default(), Visibility::Public),
                entry("secret-tool", Price::default(), Visibility::Private),
                entry("unlisted-beta", Price::default(), Visibility::Unlisted),
            ],
        };
        let hits = index.search("");
        assert_eq!(hits.len(), 1, "only public apps are searchable");
        assert_eq!(hits[0].id, "public-notes");
        // But a private app is still resolvable by exact id for install.
        assert!(index.get("secret-tool").is_some());
        assert!(index.get("unlisted-beta").is_some());
    }

    #[test]
    fn license_store_round_trips_and_gates() {
        let tmp = std::env::temp_dir().join(format!("plexi-lic-test-{}", uuid::Uuid::new_v4()));
        let store = LicenseStore::open_at(tmp.clone());
        let paid_entry = entry("pro-app", paid(), Visibility::Public);

        // No license yet → paid app needs purchase.
        assert_eq!(
            license_gate(&paid_entry, &store),
            LicenseDecision::NeedsPurchase
        );

        // A free app never needs a license.
        let free_entry = entry("free-app", Price::default(), Visibility::Public);
        assert_eq!(license_gate(&free_entry, &store), LicenseDecision::Proceed);

        // Save a license, then the paid app proceeds.
        let lic = License {
            schema_version: MARKETPLACE_SCHEMA_VERSION,
            app_id: "pro-app".to_string(),
            version: "*".to_string(),
            issued_at: "2026-06-12T00:00:00Z".to_string(),
            token: "tok_test".to_string(),
            provider: "stub".to_string(),
        };
        store.save(&lic).unwrap();
        assert_eq!(store.load("pro-app"), Some(lic));
        assert_eq!(license_gate(&paid_entry, &store), LicenseDecision::Proceed);
        assert_eq!(store.list().len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn license_version_scoping() {
        let lic = License {
            schema_version: 1,
            app_id: "a".to_string(),
            version: "1.0.0".to_string(),
            issued_at: "x".to_string(),
            token: "t".to_string(),
            provider: "stub".to_string(),
        };
        assert!(lic.authorizes("a", "1.0.0"));
        assert!(!lic.authorizes("a", "2.0.0"));
        assert!(!lic.authorizes("b", "1.0.0"));
    }

    // Factory-rule stub test: every trait method runs without panicking, and
    // purchase fails closed.
    #[test]
    fn stub_payment_provider_never_panics_and_fails_closed() {
        let provider = payment_provider();
        assert_eq!(provider.name(), "stub");
        assert!(!provider.is_configured());
        let e = entry("pro-app", paid(), Visibility::Public);
        let session = crate::app::account::AccountSession {
            schema_version: 1,
            account_id: "acct_1".to_string(),
            email: "user@example.com".to_string(),
            token: "tok".to_string(),
            provider: "stub".to_string(),
            issued_at: "2026-06-12T00:00:00Z".to_string(),
        };
        match provider.purchase(&e, &session) {
            Err(PaymentError::NotConfigured) => {}
            other => panic!("stub must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn publish_client_not_configured_fails_closed() {
        let client = PublishClient::new(None);
        let submission = Submission {
            schema_version: 1,
            entry: entry("a", Price::default(), Visibility::Public),
        };
        assert!(matches!(
            client.submit(&submission),
            Err(PublishError::NotConfigured)
        ));
        // An empty-string submit_url is also "not configured".
        let empty = PublishClient::new(Some(String::new()));
        assert!(matches!(
            empty.submit(&submission),
            Err(PublishError::NotConfigured)
        ));
    }

    #[test]
    fn registry_entry_from_report_carries_marketplace_facts() {
        let report = PackageReport {
            id: "notes".to_string(),
            name: "Notes".to_string(),
            version: "2.1.0".to_string(),
            runtime: crate::app::package::PackageRuntime::Python,
            entry: "main.py".to_string(),
            capabilities: vec![],
            file_count: 3,
            total_size: 1024,
            requires_plexi_min: None,
            requires_plexi_max: None,
        };
        let mkt = MarketplaceManifest {
            visibility: Visibility::Public,
            price: paid(),
            publisher: Some("acme".to_string()),
        };
        let entry = RegistryEntry::from_report(
            &report,
            &mkt,
            "Unreviewed Python process",
            "deadbeef".to_string(),
            vec!["productivity".to_string()],
        );
        assert_eq!(entry.id, "notes");
        assert_eq!(entry.publisher, "acme");
        assert_eq!(entry.visibility, Visibility::Public);
        assert!(!entry.price.is_free());
        assert_eq!(entry.checksum, "deadbeef");
        assert_eq!(entry.tags, vec!["productivity".to_string()]);
    }

    #[test]
    fn artifact_url_falls_back_to_checksum_addressing() {
        let client = RegistryClient::new(None, Some("https://cdn.example/pkgs".to_string()));
        let e = entry("a", Price::default(), Visibility::Public);
        assert_eq!(
            client.artifact_url(&e),
            "https://cdn.example/pkgs/abc123.plexipkg"
        );
        let mut e2 = e.clone();
        e2.package_url = Some("https://direct.example/a.plexipkg".to_string());
        assert_eq!(client.artifact_url(&e2), "https://direct.example/a.plexipkg");
    }
}
