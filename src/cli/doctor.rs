use serde::Serialize;

#[derive(Serialize)]
struct DoctorReport {
    healthy: bool,
    apps: Vec<AppReport>,
    llm_servers: Vec<LlmServerReport>,
    openrouter: OpenRouterReport,
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

/// Check whether an `openrouter-api-key` global secret is stored and, if so,
/// validate it against the OpenRouter models endpoint.
fn check_openrouter() -> OpenRouterReport {
    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{keychain_user_name, MacKeychain, SecretStore};
        let store = MacKeychain::new();
        let account = keychain_user_name("openrouter-api-key");
        match store.get(&account) {
            None => {
                log::info!("cli:doctor: openrouter-api-key not found in keychain");
                return OpenRouterReport {
                    configured: false,
                    model_count: None,
                    status: "not configured".to_string(),
                };
            }
            Some(key) => {
                log::info!("cli:doctor: openrouter-api-key found -- validating via API");
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
    #[cfg(not(target_os = "macos"))]
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

    let installed = registry.list();
    if installed.is_empty() {
        if json {
            let report = DoctorReport {
                healthy: true,
                apps: Vec::new(),
                llm_servers,
                openrouter,
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
        }
        return 0;
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

    let healthy = sick_apps.is_empty();
    let sick_count = sick_apps.len();

    if json {
        let report = DoctorReport {
            healthy,
            apps: sick_apps,
            llm_servers,
            openrouter,
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
        println!("  --> run: plexi secret set openrouter-api-key --global");
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
