//! Marketplace CLI surface — browse/search the hosted catalog, publish an app,
//! manage paid-app licenses, and gate paid installs (stint `0018`-`0021`).
//!
//! Every command here is a thin shell over `crate::app::marketplace`. The
//! invariant from the PRM holds at this layer too: nothing here is required to
//! run a locally-installed app. Browse/search need the network; if the registry
//! is unreachable they fail with a clear message and a nonzero code, but they
//! never touch installed apps. The paid-install gate fails *open* on registry
//! errors for free/local apps (registry down never blocks a local install) and
//! fails *closed* only for confirmed paid apps with no license.

use crate::app::account::AccountStore;
use crate::app::marketplace::{
    license_gate, payment_provider, LicenseDecision, LicenseStore, MarketplaceManifest,
    PaymentError, PublishClient, RegistryClient, RegistryEntry, RegistryError, Submission, Visibility,
};
use crate::app::package;
use std::path::{Path, PathBuf};

fn client() -> RegistryClient {
    RegistryClient::new(
        crate::config::marketplace_registry_url(),
        crate::config::marketplace_cdn_url(),
    )
}

/// `plexi app browse` — list every public app in the hosted catalog.
pub fn app_browse_cli() -> i32 {
    log::info!("marketplace: app browse");
    let index = match client().fetch_index() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: could not reach the marketplace: {e}");
            return 1;
        }
    };
    let listed: Vec<&RegistryEntry> = index.listed().collect();
    if listed.is_empty() {
        println!("No public apps are listed yet.");
        return 0;
    }
    println!("{} public app(s) in the marketplace:\n", listed.len());
    print_entry_table(&listed);
    crate::cli::print_tip("install one with `plexi app install <id>`.");
    0
}

/// `plexi app search <query>` — substring search over the public catalog.
pub fn app_search_cli(query: &str) -> i32 {
    log::info!("marketplace: app search query={query:?}");
    let index = match client().fetch_index() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: could not reach the marketplace: {e}");
            return 1;
        }
    };
    let hits = index.search(query);
    if hits.is_empty() {
        println!("No public apps match '{query}'.");
        return 0;
    }
    println!("{} match(es) for '{query}':\n", hits.len());
    print_entry_table(&hits);
    crate::cli::print_tip("install one with `plexi app install <id>`.");
    0
}

fn print_entry_table(entries: &[&RegistryEntry]) {
    let id_w = entries
        .iter()
        .map(|e| e.id.len())
        .max()
        .unwrap_or(3)
        .max(3);
    let ver_w = entries
        .iter()
        .map(|e| e.version.len())
        .max()
        .unwrap_or(7)
        .max(7);
    for e in entries {
        println!(
            "  {:id_w$}  {:ver_w$}  {:>8}  {}",
            e.id,
            e.version,
            e.price.display(),
            e.name,
            id_w = id_w,
            ver_w = ver_w,
        );
    }
}

/// What `install_cli` should do with a bare marketplace id after consulting the
/// hosted catalog and the license store. There is no legacy fallback — the
/// catalog is the only source for a bare id.
pub enum InstallPlan {
    /// Catalog app, free or licensed, artifact downloaded + checksum-verified.
    /// Install this `.plexipkg` directly.
    Package(PathBuf),
    /// Catalog app that resolves to a source spec (e.g. `github:owner/repo`).
    /// Install via the existing source-clone path.
    Source(String),
    /// No app with this id in the catalog. Abort with a not-found message.
    NotFound,
    /// The marketplace could not be reached. Abort — a bare id can only be
    /// resolved through the catalog. (Already-installed apps are unaffected;
    /// only *new* installs need the catalog.)
    Unreachable,
    /// Paid app with no license, and purchase is unavailable or failed. Abort.
    /// A message was already printed.
    Blocked,
}

/// Plan a bare-id install by consulting the hosted catalog. Called by
/// `install_cli`. The catalog is the sole source of truth for a bare id —
/// there is no legacy resolver and no backwards-compatibility path.
///
/// A paid app the user does not own is taken through purchase (which fails
/// closed with the stub provider). Free / licensed apps install from the CDN
/// artifact, or from a declared source spec for github-hosted catalog entries.
pub fn plan_install(app_id: &str) -> InstallPlan {
    let cli = client();
    let entry = match cli.fetch_entry(app_id) {
        Ok(e) => e,
        Err(RegistryError::NotFound(_)) => return InstallPlan::NotFound,
        Err(e) => {
            log::warn!("marketplace: catalog unreachable for '{app_id}': {e}");
            return InstallPlan::Unreachable;
        }
    };

    // Host-version pre-check: reject an incompatible app before downloading it.
    // A too-old host (or malformed requirement) blocks; a too-new host warns.
    {
        use crate::app::host_version::{check, current};
        let verdict = check(
            entry.requires_plexi_min.as_deref(),
            entry.requires_plexi_max.as_deref(),
            current(),
        );
        if let Some(msg) = verdict.message() {
            if verdict.is_blocking() {
                eprintln!("error: {msg}");
                return InstallPlan::Blocked;
            }
            eprintln!("warning: {msg}");
        }
    }

    // License gate before any download or source resolution.
    let store = LicenseStore::open();
    if license_gate(&entry, &store) == LicenseDecision::NeedsPurchase
        && !attempt_purchase(&entry, &store)
    {
        return InstallPlan::Blocked;
    }

    // Prefer a CDN artifact; fall back to a declared source spec.
    if !entry.checksum.is_empty() {
        let dest = std::env::temp_dir()
            .join(format!("plexi-mkt-{}-{}.plexipkg", entry.id, uuid::Uuid::new_v4()));
        match cli.download_package(&entry, &dest) {
            Ok(path) => return InstallPlan::Package(path),
            Err(e) => {
                eprintln!("error: could not download '{}' from the marketplace: {e}", entry.id);
                return InstallPlan::Blocked;
            }
        }
    }
    if let Some(source) = entry.source.clone() {
        return InstallPlan::Source(source);
    }
    eprintln!(
        "error: catalog entry '{app_id}' has neither a package artifact nor a source — \
         the registry entry is malformed"
    );
    InstallPlan::Blocked
}

/// Attempt to buy a paid app the user does not yet own. Requires a logged-in
/// account; today the stub payment provider always fails closed, telling the
/// user exactly why. When a real provider is wired into `payment_provider()`,
/// this issues + persists a license and returns `true` with no other change.
fn attempt_purchase(entry: &RegistryEntry, store: &LicenseStore) -> bool {
    println!("'{}' is a paid app ({}).", entry.id, entry.price.display());

    let Some(session) = AccountStore::open().current() else {
        eprintln!("error: {}", PaymentError::NoAccount);
        eprintln!("  Log in first: `plexi account login`.");
        return false;
    };

    let provider = payment_provider();
    log::info!(
        "marketplace: payment provider '{}' configured={}",
        provider.name(),
        provider.is_configured()
    );
    if !provider.is_configured() {
        eprintln!("error: {}", PaymentError::NotConfigured);
        return false;
    }
    log::info!("marketplace: purchasing '{}' as {}", entry.id, session.email);
    match provider.purchase(entry, &session) {
        Ok(license) => {
            if let Err(e) = store.save(&license) {
                eprintln!("error: purchased but could not save license: {e}");
                return false;
            }
            println!("Purchased '{}'. License saved.", entry.id);
            true
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

/// `plexi app license list` — show every stored paid-app license.
pub fn app_license_list_cli() -> i32 {
    log::info!("marketplace: license list");
    let store = LicenseStore::open();
    let licenses = store.list();
    if licenses.is_empty() {
        println!("No paid-app licenses on this machine.");
        return 0;
    }
    println!("{} license(s):\n", licenses.len());
    for lic in &licenses {
        println!(
            "  {}  v{}  (provider: {}, issued: {})",
            lic.app_id, lic.version, lic.provider, lic.issued_at
        );
    }
    0
}

/// `plexi app license show <id>` — show one license in full.
pub fn app_license_show_cli(id: &str) -> i32 {
    log::info!("marketplace: license show id={id}");
    let store = LicenseStore::open();
    match store.load(id) {
        Some(lic) => {
            println!("app:       {}", lic.app_id);
            println!("version:   {}", lic.version);
            println!("provider:  {}", lic.provider);
            println!("issued_at: {}", lic.issued_at);
            println!("token:     {}", lic.token);
            if !lic.signature.is_empty() {
                println!("signature: {}", lic.signature);
            }
            0
        }
        None => {
            eprintln!("no license stored for '{id}'");
            1
        }
    }
}

/// `plexi app publish [<path>]` — validate, package, and submit an app to the
/// hosted marketplace. Replaces the old print-only stub.
///
/// Always does the real local work: reads `[marketplace]`, validates the dir,
/// builds the `.plexipkg`, checksums it, and assembles the submission. The
/// upload is the only part gated on configuration — without
/// `[marketplace].submit_url` it reports the prepared submission and where the
/// artifact is, then exits 0. This is the honest "last mile not wired" boundary,
/// not a silent no-op.
pub fn app_publish_cli(path: &str) -> i32 {
    log::info!("marketplace: app publish path={path}");
    let app_dir = match Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };

    // Read the marketplace manifest section (publisher/visibility/price).
    let marketplace = match read_marketplace_manifest(&app_dir) {
        Ok(m) => m,
        Err(code) => return code,
    };

    // Validate the directory (fail-closed) — this is the same gate publish
    // review will run server-side.
    let report = match package::validate_dir(&app_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: validation failed — refusing to publish: {e}");
            return 1;
        }
    };

    // Build the distributable artifact and checksum it.
    let artifact = match package::build_package(&app_dir, None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not build package: {e}");
            return 1;
        }
    };
    let checksum = match package::sha256_file(&artifact) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: could not checksum package: {e}");
            return 1;
        }
    };

    let core_ids = crate::cli::install_host::core_pack_ids();
    let core_refs: Vec<&str> = core_ids.iter().map(String::as_str).collect();
    let trust = package::trust_label(&report, &core_refs);

    let entry = RegistryEntry::from_report(
        &report,
        &marketplace,
        trust.display_str(),
        checksum,
        Vec::new(),
    );
    let submission = Submission {
        schema_version: crate::app::marketplace::MARKETPLACE_SCHEMA_VERSION,
        entry: entry.clone(),
    };

    println!("Prepared submission for '{}' v{}:", entry.id, entry.version);
    println!("  publisher:  {}", entry.publisher);
    println!("  visibility: {}", entry.visibility.as_str());
    println!("  price:      {}", entry.price.display());
    if entry.requires_plexi_min.is_some() || entry.requires_plexi_max.is_some() {
        let min = entry.requires_plexi_min.as_deref().unwrap_or("any");
        let max = entry.requires_plexi_max.as_deref().unwrap_or("any");
        println!("  requires:   Plexi {min} .. {max}");
    }
    println!("  trust:      {}", entry.trust_label);
    println!("  artifact:   {}", artifact.display());
    println!("  checksum:   {}", entry.checksum);

    let publisher = PublishClient::new(crate::config::marketplace_submit_url());
    match publisher.submit(&submission) {
        Ok(state) => {
            println!("\nSubmitted to the marketplace (state: {state:?}).");
            0
        }
        Err(crate::app::marketplace::PublishError::NotConfigured) => {
            println!(
                "\nNo submission endpoint configured — the package was validated and prepared but \
                 not uploaded."
            );
            println!(
                "Set [marketplace].submit_url in your config to enable upload, or share the \
                 artifact above directly."
            );
            crate::cli::print_tip("follow marketplace progress at https://plexiapp.com/docs/marketplace");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Read and validate the `[marketplace]` manifest section for publishing.
/// Returns the section, or an exit code on a publish-blocking problem.
fn read_marketplace_manifest(app_dir: &Path) -> Result<MarketplaceManifest, i32> {
    let manifest_path = app_dir.join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: could not read {}: {e}", manifest_path.display());
            return Err(1);
        }
    };
    let manifest: crate::app::registry::AppManifest = match toml::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: manifest.toml parse failed: {e}");
            return Err(1);
        }
    };
    let Some(marketplace) = manifest.marketplace else {
        eprintln!(
            "error: no [marketplace] section in manifest.toml.\n  \
             Add one to publish, e.g.:\n\n  [marketplace]\n  visibility = \"public\"\n  \
             price = \"free\"\n  publisher = \"your-org\""
        );
        return Err(1);
    };
    if marketplace.publisher.as_deref().unwrap_or("").is_empty() {
        eprintln!("error: [marketplace].publisher is required to publish");
        return Err(1);
    }
    if marketplace.visibility == Visibility::Private {
        eprintln!(
            "warning: [marketplace].visibility is \"private\" — this app will be submitted but \
             not publicly listed. Use \"public\" to list it, \"unlisted\" for install-by-id only."
        );
    }
    Ok(marketplace)
}
