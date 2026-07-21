//! Marketplace CLI surface — browse/search the hosted catalog, publish an app,
//! and plan bare-id installs (stints `0018`-`0021`, `0340`).
//!
//! Every command here is a thin shell over `crate::app::marketplace`. The
//! invariant from the PRM holds at this layer too: nothing here is required to
//! run a locally-installed app. Browse/search need the network; if the registry
//! is unreachable they fail with a clear message and a nonzero code, but they
//! never touch installed apps. Paid apps are bought through the marketplace
//! (browser checkout); the CLI never charges and there is no client-side
//! license — a bare CLI install of a paid app is blocked with a pointer.

use crate::app::marketplace::{
    InstalledRegistrySource, MarketplaceManifest, PublishClient, RegistryClient, RegistryEntry,
    RegistryError, Submission, Visibility,
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
    let id_w = entries.iter().map(|e| e.id.len()).max().unwrap_or(3).max(3);
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
    /// Free catalog app, artifact downloaded + checksum-verified. Install this
    /// `.plexipkg` directly.
    Package {
        path: PathBuf,
        reviewed_native: bool,
        source_metadata: InstalledRegistrySource,
    },
    /// Catalog app that resolves to a source spec (e.g. `github:owner/repo`).
    /// Install via the existing source-clone path.
    Source(String),
    /// No app with this id in the catalog. Abort with a not-found message.
    NotFound,
    /// The marketplace could not be reached. Abort — a bare id can only be
    /// resolved through the catalog. (Already-installed apps are unaffected;
    /// only *new* installs need the catalog.)
    Unreachable,
    /// The install cannot proceed (paid app, malformed entry, or download
    /// failure). A message was already printed.
    Blocked,
}

/// Plan a bare-id install by consulting the hosted catalog. Called by
/// `install_cli`. The catalog is the sole source of truth for a bare id —
/// there is no legacy resolver and no backwards-compatibility path.
///
/// Paid apps are blocked here: they are bought through the marketplace (browser
/// checkout), and the server gates the paid artifact download on the account's
/// purchase row. Free apps install from the CDN artifact, or from a declared
/// source spec for github-hosted catalog entries.
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

    // Paid apps are purchased through the marketplace, never the CLI: the server
    // gates the paid artifact download on the account's purchase row. Block a
    // bare CLI install of a paid app with a clear pointer. Free apps proceed.
    if !entry.price.is_free() {
        log::info!(
            "marketplace: bare install of paid app '{app_id}' blocked (buy via marketplace)"
        );
        eprintln!(
            "error: '{}' is a paid app ({}). Buy it through the marketplace — open the app in \
             Plexi or visit https://plexiapp.com — then it installs for your account. \
             Log in first with `plexi account login`.",
            entry.id,
            entry.price.display()
        );
        return InstallPlan::Blocked;
    }

    // Prefer a CDN artifact; fall back to a declared source spec.
    if !entry.checksum.is_empty() {
        let dest = std::env::temp_dir().join(format!(
            "plexi-mkt-{}-{}.plexipkg",
            entry.id,
            uuid::Uuid::new_v4()
        ));
        match cli.download_package(&entry, &dest) {
            Ok(path) => {
                return InstallPlan::Package {
                    path,
                    reviewed_native: entry.reviewed_native,
                    source_metadata: cli.installed_source_metadata(&entry),
                }
            }
            Err(e) => {
                eprintln!(
                    "error: could not download '{}' from the marketplace: {e}",
                    entry.id
                );
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
    let trust = package::trust_label(&report, &core_refs, false);

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
            crate::cli::print_tip(
                "follow marketplace progress at https://plexiapp.com/docs/marketplace",
            );
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
