use serde::Serialize;

#[derive(Serialize)]
struct DoctorReport {
    healthy: bool,
    apps: Vec<AppReport>,
    llm_servers: Vec<LlmServerReport>,
    openrouter: OpenRouterReport,
    version: VersionSkewReport,
}

/// Install/running-host version visibility (stint 0596): the on-disk bundle
/// version vs. what a running host process was actually launched with, plus
/// whether the last install skipped the CLI shim/completions.
#[derive(Serialize)]
struct VersionSkewReport {
    /// `None` when `installed_tag` is missing/empty/unreadable — there is no
    /// `CARGO_PKG_VERSION` fallback (that's the CLI shim's own compiled
    /// version, not the bundle's; see `read_bundle_version`).
    bundle_version: Option<String>,
    host_version: Option<String>,
    /// `true` when the running host is strictly older than the bundle
    /// (restart would apply a newer version); `None` when no host is
    /// running or either version failed to parse.
    skewed: Option<bool>,
    shim_updated: Option<bool>,
    /// Human-readable status string for non-JSON output.
    #[serde(skip)]
    status: String,
    /// Human-readable shim remedy line for non-JSON output, when the last
    /// install skipped or could not confirm the shim install.
    #[serde(skip)]
    shim_note: Option<String>,
}

/// Build the version-skew report by reusing `src/cli/host.rs`'s
/// read/query helpers rather than re-deriving profile paths here.
fn check_version_skew() -> VersionSkewReport {
    let channel = crate::config::build_channel();
    let bundle_version = crate::cli::host::read_bundle_version(channel.as_deref());
    let host_version = crate::cli::host::query_running_host_version(channel.as_deref());
    let shim_status = crate::cli::host::read_shim_status(channel.as_deref());

    let skew: Option<crate::cli::release_resolver::SkewStatus> = match &bundle_version {
        Err(reason) => Some(crate::cli::release_resolver::SkewStatus::Unknown {
            reason: reason.clone(),
        }),
        Ok(bundle) => host_version
            .as_deref()
            .map(|running| crate::cli::release_resolver::detect_version_skew(bundle, running)),
    };

    let (skewed, status) = match &skew {
        Some(crate::cli::release_resolver::SkewStatus::InSync) => (
            Some(false),
            format!(
                "{} (bundle and running host match)",
                bundle_version.as_ref().unwrap().trim_start_matches('v')
            ),
        ),
        Some(crate::cli::release_resolver::SkewStatus::Skewed { bundle, running }) => {
            log::info!(
                "cli:doctor: version skew detected — bundle={bundle} running_host={running}"
            );
            (
                Some(true),
                format!(
                    "SKEW: bundle {} vs running host {} — restart Plexi to apply",
                    bundle.trim_start_matches('v'),
                    running.trim_start_matches('v')
                ),
            )
        }
        Some(crate::cli::release_resolver::SkewStatus::Unknown { reason }) => {
            (None, format!("unknown ({reason})"))
        }
        None => (
            None,
            format!(
                "{} (host not running)",
                bundle_version.as_ref().unwrap().trim_start_matches('v')
            ),
        ),
    };

    let (shim_updated, shim_note) = match &shim_status {
        crate::cli::host::ShimStatusInfo::Ok(s) if !s.shim_updated => {
            let reason = s.reason.as_deref().unwrap_or("unknown reason");
            (
                Some(false),
                Some(format!(
                    "the last install did not update the CLI shim/completions ({reason}) — run 'plexi update' in a terminal to install the CLI shim"
                )),
            )
        }
        crate::cli::host::ShimStatusInfo::Ok(s) => (Some(s.shim_updated), None),
        crate::cli::host::ShimStatusInfo::Malformed => (
            None,
            Some(
                "shim install status is unknown (malformed shim_status.json) — run 'plexi update' in a terminal to install the CLI shim"
                    .to_string(),
            ),
        ),
        crate::cli::host::ShimStatusInfo::Absent => (None, None),
    };

    VersionSkewReport {
        bundle_version: bundle_version.ok(),
        host_version,
        skewed,
        shim_updated,
        status,
        shim_note,
    }
}

#[derive(Serialize)]
struct OpenRouterReport {
    configured: bool,
    /// Number of models returned by OpenRouter's /v1/models endpoint.
    /// None if the key is absent or the API call failed.
    model_count: Option<usize>,
    /// Human-readable status string for non-JSON output.
    #[serde(skip)]
    status: String,
}

#[derive(Serialize)]
struct AppReport {
    id: String,
    missing: Vec<String>,
}

#[derive(Serialize)]
struct LlmServerReport {
    name: String,
    url: String,
    models: Vec<String>,
}

/// Check whether an `OPENROUTER_API_KEY` global secret is stored and, if so,
/// validate it against the OpenRouter models endpoint.
fn check_openrouter() -> OpenRouterReport {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        use crate::workspace::secrets::{keychain_user_name, system_store};
        let store = system_store();
        let key = ["OPENROUTER_API_KEY", "openrouter-api-key"]
            .iter()
            .find_map(|name| store.get(&keychain_user_name(name)));
        match key {
            None => {
                log::info!("cli:doctor: OPENROUTER_API_KEY not found in keychain");
                OpenRouterReport {
                    configured: false,
                    model_count: None,
                    status: "not configured".to_string(),
                }
            }
            Some(key) => {
                log::info!("cli:doctor: OPENROUTER_API_KEY found -- validating via API");
                match probe_openrouter_key(key.as_str()) {
                    Some(count) => {
                        log::info!("cli:doctor: openrouter key valid -- {count} models");
                        OpenRouterReport {
                            configured: true,
                            model_count: Some(count),
                            status: format!("configured ({count} models available)"),
                        }
                    }
                    None => {
                        log::warn!("cli:doctor: openrouter key present but API validation failed");
                        OpenRouterReport {
                            configured: true,
                            model_count: None,
                            status: "configured (key set, API unreachable)".to_string(),
                        }
                    }
                }
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        log::info!(
            "cli:doctor: openrouter check skipped -- keychain not available on this platform"
        );
        OpenRouterReport {
            configured: false,
            model_count: None,
            status: "not configured (keychain unavailable)".to_string(),
        }
    }
}

/// Validate an OpenRouter API key by calling GET /api/v1/models.
/// Returns the number of models on success, or None if the call fails.
fn probe_openrouter_key(api_key: &str) -> Option<usize> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_millis(3000))
        .timeout(std::time::Duration::from_secs(5))
        .build();

    let resp = match agent
        .get("https://openrouter.ai/api/v1/models")
        .set("Authorization", &format!("Bearer {api_key}"))
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            log::debug!("cli:doctor: openrouter probe failed: {e}");
            return None;
        }
    };

    let text = match resp.into_string() {
        Ok(t) => t,
        Err(e) => {
            log::debug!("cli:doctor: openrouter probe read error: {e}");
            return None;
        }
    };

    let body: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("cli:doctor: openrouter probe bad JSON: {e}");
            return None;
        }
    };

    // OpenRouter /v1/models returns {"data":[{"id":"..."},...]}
    body["data"].as_array().map(|arr| arr.len())
}

/// Probe a single LLM server endpoint.
/// Returns `Some(models)` if the server is reachable and returns a parseable model list.
fn probe_llm_server(url: &str) -> Option<Vec<String>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_millis(1500))
        .timeout(std::time::Duration::from_secs(2))
        .build();

    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(e) => {
            log::debug!("cli:doctor: llm probe {url} -- not reachable: {e}");
            return None;
        }
    };

    let text = match resp.into_string() {
        Ok(t) => t,
        Err(e) => {
            log::debug!("cli:doctor: llm probe {url} -- read error: {e}");
            return None;
        }
    };

    let body: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("cli:doctor: llm probe {url} -- bad JSON: {e}");
            return None;
        }
    };

    // Ollama /api/tags returns {"models":[{"name":"..."},...]}
    // OpenAI-compatible /v1/models returns {"data":[{"id":"..."},...]}
    let models: Vec<String> = if let Some(arr) = body["models"].as_array() {
        arr.iter()
            .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
            .collect()
    } else if let Some(arr) = body["data"].as_array() {
        arr.iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect()
    } else {
        Vec::new()
    };

    Some(models)
}

/// Probe all well-known local LLM server endpoints and return discovered servers.
fn discover_llm_servers() -> Vec<LlmServerReport> {
    let candidates = [
        ("Ollama", "http://localhost:11434", "/api/tags"),
        ("LM Studio", "http://localhost:1234", "/v1/models"),
        ("llama.cpp", "http://localhost:8080", "/v1/models"),
    ];

    let mut found = Vec::new();
    for (name, base, path) in &candidates {
        let url = format!("{base}{path}");
        if let Some(models) = probe_llm_server(&url) {
            log::info!(
                "cli:doctor: found local LLM server -- name={name} url={base} models={}",
                models.len()
            );
            found.push(LlmServerReport {
                name: name.to_string(),
                url: base.to_string(),
                models,
            });
        }
    }
    found
}

pub fn doctor_cli(json: bool) -> i32 {
    log::info!("cli:doctor: starting capability audit (json={json})");

    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app::registry::AppRegistry::load(&cwd);
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );

    // Probe for local LLM servers and OpenRouter before the app audit.
    let llm_servers = discover_llm_servers();
    let openrouter = check_openrouter();
    let version = check_version_skew();
    let version_healthy = version.skewed != Some(true);

    let installed = registry.list();
    if installed.is_empty() {
        let healthy = version_healthy;
        if json {
            let report = DoctorReport {
                healthy,
                apps: Vec::new(),
                llm_servers,
                openrouter,
                version,
            };
            match serde_json::to_string_pretty(&report) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("error: failed to serialize doctor report: {e}");
                    return 1;
                }
            }
        } else {
            println!("No apps installed.");
            print_llm_section(&llm_servers);
            print_openrouter_section(&openrouter);
            print_version_section(&version);
        }
        return if healthy { 0 } else { 1 };
    }

    let mut sick_apps: Vec<AppReport> = Vec::new();
    let total = installed.len();

    for app in &installed {
        let missing = registry.check_config_capabilities(&app.manifest.id, &config);
        if !missing.is_empty() {
            sick_apps.push(AppReport {
                id: app.manifest.id.clone(),
                missing,
            });
        }
    }

    let healthy = sick_apps.is_empty() && version_healthy;
    let sick_count = sick_apps.len();

    if json {
        let report = DoctorReport {
            healthy,
            apps: sick_apps,
            llm_servers,
            openrouter,
            version,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: failed to serialize doctor report: {e}");
                return 1;
            }
        }
    } else {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let green = if no_color { "" } else { "\x1b[32m" };
        let red = if no_color { "" } else { "\x1b[31m" };
        let dim = if no_color { "" } else { "\x1b[2m" };
        let reset = if no_color { "" } else { "\x1b[0m" };

        println!("Checking {total} installed app(s)...\n");

        if healthy {
            println!("  {green}\u{2713}{reset} {total} app(s) -- all capabilities satisfied");
        } else {
            let ok_count = total - sick_count;
            if ok_count > 0 {
                println!(
                    "  {green}\u{2713}{reset} {ok_count} app(s) -- all capabilities satisfied"
                );
            }
            for app in &sick_apps {
                let first = &app.missing[0];
                println!("  {red}\u{2717}{reset} {:12} -- {first}", app.id);
                for reason in app.missing.iter().skip(1) {
                    println!("  {:14} {dim}-- {reason}{reset}", "");
                }
                println!("  {:14} {dim}--> run: plexi config edit{reset}", "");
            }
            println!(
                "\n{sick_count} app(s) have unsatisfied capabilities. Run 'plexi config edit' to fix."
            );
        }

        print_llm_section(&llm_servers);
        print_openrouter_section(&openrouter);
        print_version_section(&version);
    }

    log::info!("cli:doctor: audit complete -- {total} app(s), {sick_count} unhealthy");

    if healthy {
        0
    } else {
        1
    }
}

/// Print the OpenRouter section to stdout.
fn print_openrouter_section(report: &OpenRouterReport) {
    println!("\nOpenRouter:");
    println!("  {}", report.status);
    if !report.configured {
        println!("  --> run: plexi secret set OPENROUTER_API_KEY --global");
    }
}

/// Print the version-skew section to stdout (stint 0596).
fn print_version_section(report: &VersionSkewReport) {
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let green = if no_color { "" } else { "\x1b[32m" };
    let red = if no_color { "" } else { "\x1b[31m" };
    let dim = if no_color { "" } else { "\x1b[2m" };
    let reset = if no_color { "" } else { "\x1b[0m" };

    println!("\nVersion:");
    match report.skewed {
        Some(true) => println!("  {red}\u{2717}{reset} {}", report.status),
        _ => println!("  {green}\u{2713}{reset} {}", report.status),
    }
    if let Some(note) = &report.shim_note {
        println!("  {dim}--> {note}{reset}");
    }
}

/// Print the local LLM servers section to stdout.
fn print_llm_section(servers: &[LlmServerReport]) {
    println!("\nLocal LLM servers:");
    if servers.is_empty() {
        println!("  No local LLM servers detected");
    } else {
        for s in servers {
            let count = s.models.len();
            let model_summary = if count == 0 {
                "(no models listed)".to_string()
            } else if count <= 3 {
                s.models.join(", ")
            } else {
                format!("{}, ... ({} total)", s.models[..3].join(", "), count)
            };
            println!("  {} ({}) -- {}", s.name, s.url, model_summary);
        }
    }
}
