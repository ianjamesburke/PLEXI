use serde::Serialize;

// ── Hardware detection ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HardwareReport {
    cpu_name: Option<String>,
    cpu_cores: Option<u32>,
    ram_gb: Option<f64>,
    gpu_name: Option<String>,
    /// On Apple Silicon, VRAM == unified RAM; on discrete GPU, this is the card's VRAM.
    vram_gb: Option<f64>,
    is_apple_silicon: bool,
    disk_free_gb: Option<f64>,
}

#[derive(Serialize)]
struct IntegrationReport {
    ollama_installed: bool,
    ollama_running: bool,
    ollama_models: Vec<String>,
    openrouter_configured: bool,
}

#[derive(Serialize)]
struct ModelRecommendation {
    tier: &'static str,
    models: Vec<&'static str>,
    note: &'static str,
}

#[derive(Serialize)]
struct AiDoctorReport {
    hardware: HardwareReport,
    integrations: IntegrationReport,
    recommendation: ModelRecommendation,
}

fn detect_hardware() -> HardwareReport {
    let cpu_name = run_sysctl("machdep.cpu.brand_string");
    let cpu_cores = run_sysctl("hw.ncpu").and_then(|s| s.trim().parse::<u32>().ok());
    let ram_bytes = run_sysctl("hw.memsize").and_then(|s| s.trim().parse::<u64>().ok());
    let ram_gb = ram_bytes.map(|b| b as f64 / (1024.0 * 1024.0 * 1024.0));

    // Apple Silicon: arm64 CPU brand string contains "Apple"
    let is_apple_silicon = cpu_name
        .as_deref()
        .map(|n| n.contains("Apple"))
        .unwrap_or(false);

    let (gpu_name, vram_gb) = detect_gpu(is_apple_silicon, ram_gb);

    let disk_free_gb = detect_disk_free();

    HardwareReport {
        cpu_name,
        cpu_cores,
        ram_gb,
        gpu_name,
        vram_gb,
        is_apple_silicon,
        disk_free_gb,
    }
}

fn run_sysctl(key: &str) -> Option<String> {
    let out = std::process::Command::new("sysctl")
        .arg("-n")
        .arg(key)
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

fn detect_gpu(is_apple_silicon: bool, ram_gb: Option<f64>) -> (Option<String>, Option<f64>) {
    // Try system_profiler for GPU name; unified memory is the VRAM on Apple Silicon.
    let out = std::process::Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .arg("-json")
        .output();

    let gpu_name = match out {
        Ok(ref o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            parse_gpu_name_from_profiler(&text)
        }
        _ => None,
    };

    let vram_gb = if is_apple_silicon {
        // On Apple Silicon, unified memory is the effective VRAM.
        ram_gb
    } else {
        // Try to parse VRAM from system_profiler JSON.
        if let Ok(ref o) = out {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                parse_vram_from_profiler(&text)
            } else {
                None
            }
        } else {
            None
        }
    };

    (gpu_name, vram_gb)
}

/// Parse the GPU name from `system_profiler SPDisplaysDataType -json` output.
/// The JSON shape is: {"SPDisplaysDataType": [{"sppci_model": "Apple M3 Pro", ...}]}
fn parse_gpu_name_from_profiler(json: &str) -> Option<String> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;
    val["SPDisplaysDataType"].as_array()?.first()?["sppci_model"]
        .as_str()
        .map(|s| s.to_string())
}

/// Parse discrete VRAM from system_profiler JSON.
/// Field is "spdisplays_vram" — value like "8192 MB" or "16 GB".
fn parse_vram_from_profiler(json: &str) -> Option<f64> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;
    let vram_str = val["SPDisplaysDataType"].as_array()?.first()?["spdisplays_vram"].as_str()?;

    // Parse "8192 MB" or "16 GB"
    let vram_str = vram_str.trim();
    if let Some(mb) = vram_str.strip_suffix(" MB") {
        mb.trim().parse::<f64>().ok().map(|v| v / 1024.0)
    } else if let Some(gb) = vram_str.strip_suffix(" GB") {
        gb.trim().parse::<f64>().ok()
    } else {
        None
    }
}

fn detect_disk_free() -> Option<f64> {
    // Use statvfs via std::fs metadata on the home directory.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let meta = std::fs::metadata(&home).ok()?;
        // statvfs not directly accessible via std; use df -k as a fallback.
        let _ = meta.dev();
        detect_disk_free_via_df(&home)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn detect_disk_free_via_df(path: &str) -> Option<f64> {
    let out = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // df -k output: Filesystem 1K-blocks Used Available Capacity Mounted on
    // Second line has the values.
    let line = text.lines().nth(1)?;
    let parts: Vec<&str> = line.split_whitespace().collect();
    // Available is column 3 (0-indexed)
    let available_kb: u64 = parts.get(3)?.parse().ok()?;
    Some(available_kb as f64 / (1024.0 * 1024.0)) // convert KB to GB
}

// ── Integration checks ────────────────────────────────────────────────────────

fn check_integrations() -> IntegrationReport {
    let ollama_installed = crate::cli::binary_in_path("ollama")
        || std::path::Path::new("/usr/local/bin/ollama").exists()
        || std::path::Path::new("/opt/homebrew/bin/ollama").exists();

    let (ollama_running, ollama_models) = probe_ollama();
    let openrouter_configured = check_openrouter_configured();

    IntegrationReport {
        ollama_installed,
        ollama_running,
        ollama_models,
        openrouter_configured,
    }
}

fn probe_ollama() -> (bool, Vec<String>) {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_millis(1000))
        .timeout(std::time::Duration::from_secs(2))
        .build();

    let resp = match agent.get("http://localhost:11434/api/tags").call() {
        Ok(r) => r,
        Err(e) => {
            log::debug!("ai:doctor: ollama probe failed: {e}");
            return (false, Vec::new());
        }
    };

    let text = match resp.into_string() {
        Ok(t) => t,
        Err(e) => {
            log::debug!("ai:doctor: ollama probe read error: {e}");
            return (true, Vec::new());
        }
    };

    let body: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("ai:doctor: ollama probe bad JSON: {e}");
            return (true, Vec::new());
        }
    };

    let models: Vec<String> = body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    (true, models)
}

fn check_openrouter_configured() -> bool {
    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{keychain_user_name, MacKeychain, SecretStore};
        let store = MacKeychain::new();
        let account = keychain_user_name("openrouter-api-key");
        store.get(&account).is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

// ── Model recommendation ──────────────────────────────────────────────────────

fn recommend_models(hw: &HardwareReport) -> ModelRecommendation {
    let ram = hw.vram_gb.unwrap_or(hw.ram_gb.unwrap_or(0.0));

    if hw.is_apple_silicon && ram >= 32.0 {
        ModelRecommendation {
            tier: "local-great",
            models: vec!["llama3.1:8b", "llama3.2:3b", "mistral:7b", "llama3.1:70b-q4"],
            note: "Your Apple Silicon Mac has enough unified memory for most local models. llama3.1:8b is the sweet spot; llama3.1:70b-q4 is possible but slower.",
        }
    } else if hw.is_apple_silicon && ram >= 16.0 {
        ModelRecommendation {
            tier: "local-great",
            models: vec!["llama3.2:3b", "llama3.1:8b", "mistral:7b"],
            note: "Your Apple Silicon Mac runs 3B-8B models well. llama3.2:3b is the fastest; llama3.1:8b gives better quality.",
        }
    } else if hw.is_apple_silicon && ram >= 8.0 {
        ModelRecommendation {
            tier: "local-ok",
            models: vec!["llama3.2:3b", "qwen2.5:3b"],
            note: "Small models (3B) will run. Larger models will be slow or fail. Cloud via OpenRouter is recommended for better throughput.",
        }
    } else if !hw.is_apple_silicon && ram >= 8.0 {
        // Intel/AMD with CUDA or no discrete GPU
        ModelRecommendation {
            tier: "local-ok",
            models: vec!["llama3.2:3b", "llama3.1:8b"],
            note: "Non-Apple hardware with adequate RAM. Performance depends on GPU. Cloud via OpenRouter may be faster.",
        }
    } else {
        ModelRecommendation {
            tier: "cloud-recommended",
            models: vec!["openrouter/anthropic/claude-3-haiku", "openrouter/meta-llama/llama-3.1-8b-instruct:free"],
            note: "Your hardware is best suited for cloud models via OpenRouter. Free tier models are available.",
        }
    }
}

// ── Output formatting ─────────────────────────────────────────────────────────

fn print_report(hw: &HardwareReport, integrations: &IntegrationReport, rec: &ModelRecommendation) {
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let green = if no_color { "" } else { "\x1b[32m" };
    let yellow = if no_color { "" } else { "\x1b[33m" };
    let red = if no_color { "" } else { "\x1b[31m" };
    let bold = if no_color { "" } else { "\x1b[1m" };
    let dim = if no_color { "" } else { "\x1b[2m" };
    let reset = if no_color { "" } else { "\x1b[0m" };

    println!("{bold}Hardware{reset}");

    if let Some(ref cpu) = hw.cpu_name {
        println!("  CPU:       {cpu}");
    }
    if let Some(cores) = hw.cpu_cores {
        println!("  Cores:     {cores}");
    }
    if let Some(ram) = hw.ram_gb {
        println!(
            "  RAM:       {:.1} GB{}",
            ram,
            if hw.is_apple_silicon {
                " (unified)"
            } else {
                ""
            }
        );
    }
    if let Some(ref gpu) = hw.gpu_name {
        println!("  GPU:       {gpu}");
    }
    if let Some(vram) = hw.vram_gb {
        let label = if hw.is_apple_silicon {
            "VRAM (unified):"
        } else {
            "VRAM:          "
        };
        println!("  {label} {vram:.1} GB");
    }
    if let Some(disk) = hw.disk_free_gb {
        let disk_color = if disk >= 20.0 {
            green
        } else if disk >= 5.0 {
            yellow
        } else {
            red
        };
        println!("  Disk free: {disk_color}{disk:.1} GB{reset}");
    }

    println!();
    println!("{bold}Integrations{reset}");

    let ok = format!("{green}\u{2713}{reset}");
    let no = format!("{red}\u{2717}{reset}");
    let warn = format!("{yellow}!{reset}");

    let ollama_status = if integrations.ollama_running {
        let count = integrations.ollama_models.len();
        format!("{ok} Ollama running — {count} model(s) pulled")
    } else if integrations.ollama_installed {
        format!("{warn} Ollama installed but not running")
    } else {
        format!("{no} Ollama not installed  {dim}(brew install ollama){reset}")
    };
    println!("  {ollama_status}");

    if integrations.ollama_running && !integrations.ollama_models.is_empty() {
        let model_list = if integrations.ollama_models.len() <= 4 {
            integrations.ollama_models.join(", ")
        } else {
            format!(
                "{}, ... ({} total)",
                integrations.ollama_models[..4].join(", "),
                integrations.ollama_models.len()
            )
        };
        println!("    {dim}{model_list}{reset}");
    }

    let openrouter_status = if integrations.openrouter_configured {
        format!("{ok} OpenRouter API key configured")
    } else {
        format!("{no} OpenRouter not configured  {dim}(plexi secret set openrouter-api-key --global){reset}")
    };
    println!("  {openrouter_status}");

    println!();
    println!("{bold}Model Recommendation{reset}");

    let tier_label = match rec.tier {
        "local-great" => format!("{green}local — runs great{reset}"),
        "local-ok" => format!("{yellow}local — runs but may be slow{reset}"),
        _ => format!("{yellow}cloud recommended{reset}"),
    };
    println!("  Tier:    {tier_label}");
    println!("  Models:  {}", rec.models.join(", "));
    println!("  {dim}{}{reset}", rec.note);
}

// ── Setup wizard ─────────────────────────────────────────────────────────────

/// Write or update the `[ai]` + `[ai.ollama]` section in config.toml.
///
/// Strategy: read the existing file as raw text, then:
/// - If an `[ai]` block already exists, replace it.
/// - Otherwise, append the block at the end.
///
/// This preserves all other sections and comments.
fn write_ollama_config(model: &str) -> std::io::Result<()> {
    let path = crate::config::config_path();

    // Ensure parent dir exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let snippet = format!(
        "\n[ai]\nbackend = \"ollama\"\n\n[ai.ollama]\nhost         = \"http://localhost:11434\"\nmodel_low    = \"{model}\"\nmodel_medium = \"{model}\"\nmodel_high   = \"{model}\"\n"
    );

    if path.exists() {
        let existing = std::fs::read_to_string(&path)?;

        // Check if [ai] block already present — strip it out, then append.
        let stripped = strip_ai_section(&existing);
        let new_content = format!("{}\n{}", stripped.trim_end(), snippet);
        std::fs::write(&path, new_content)?;
    } else {
        // Config doesn't exist — seed from template, then append.
        let mut content = crate::config::CONFIG_TEMPLATE.to_string();
        content.push_str(&snippet);
        std::fs::write(&path, content)?;
    }

    Ok(())
}

/// Remove any existing `[ai]` and `[ai.*]` sections from raw TOML text.
/// Returns the text with those sections stripped so we can append fresh ones.
fn strip_ai_section(text: &str) -> String {
    let mut result = Vec::new();
    let mut skipping = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[ai]" || trimmed.starts_with("[ai.") {
            skipping = true;
            continue;
        }
        // A new top-level section (not ai) ends the skip.
        if skipping && trimmed.starts_with('[') && !trimmed.starts_with("[ai") {
            skipping = false;
        }
        if !skipping {
            result.push(line);
        }
    }

    result.join("\n")
}

pub fn ai_setup_cli() -> i32 {
    log::info!("cli:ai:setup: starting local model wizard");

    let no_color = std::env::var_os("NO_COLOR").is_some();
    let green = if no_color { "" } else { "\x1b[32m" };
    let yellow = if no_color { "" } else { "\x1b[33m" };
    let red = if no_color { "" } else { "\x1b[31m" };
    let bold = if no_color { "" } else { "\x1b[1m" };
    let dim = if no_color { "" } else { "\x1b[2m" };
    let reset = if no_color { "" } else { "\x1b[0m" };

    println!("{bold}plexi ai setup{reset} — local model wizard");
    println!();

    // ── Step 1: detect hardware ───────────────────────────────────────────────
    println!("Scanning hardware...");
    let hw = detect_hardware();
    let rec = recommend_models(&hw);
    log::info!(
        "cli:ai:setup: hardware -- ram_gb={:?} is_apple_silicon={} tier={}",
        hw.ram_gb,
        hw.is_apple_silicon,
        rec.tier
    );

    if rec.tier == "cloud-recommended" {
        println!("{yellow}Your hardware is better suited for cloud models.{reset}");
        println!("  {dim}{}{reset}", rec.note);
        println!();
        println!("To configure cloud AI instead, run:");
        println!("  {dim}plexi secret set openrouter-api-key --global{reset}");
        crate::cli::print_tip(
            "Use `plexi ai doctor` to see your full hardware and integration report.",
        );
        return 0;
    }

    let recommended_model = rec.models.first().copied().unwrap_or("llama3.2:3b");
    log::info!("cli:ai:setup: recommended model={recommended_model}");

    println!(
        "Hardware: {}",
        hw.cpu_name.as_deref().unwrap_or("unknown CPU")
    );
    if let Some(ram) = hw.ram_gb {
        println!(
            "RAM:      {ram:.1} GB{}",
            if hw.is_apple_silicon {
                " (unified)"
            } else {
                ""
            }
        );
    }
    println!("Recommended model: {green}{recommended_model}{reset}");
    println!("  {dim}{}{reset}", rec.note);
    println!();

    // ── Step 2: check Ollama installation ─────────────────────────────────────
    let ollama_installed = crate::cli::binary_in_path("ollama")
        || std::path::Path::new("/usr/local/bin/ollama").exists()
        || std::path::Path::new("/opt/homebrew/bin/ollama").exists();

    log::info!("cli:ai:setup: ollama_installed={ollama_installed}");

    if !ollama_installed {
        println!("{bold}Step 1/3: Install Ollama{reset}");
        println!("  Ollama is not installed.");
        println!();
        println!("  Install with Homebrew:");
        println!("    {dim}brew install ollama{reset}");
        println!();
        println!("  Or with the official installer:");
        println!("    {dim}curl -fsSL https://ollama.com/install.sh | sh{reset}");
        println!();
        println!("{yellow}Re-run `plexi ai setup` after installing Ollama.{reset}");
        crate::cli::print_tip("After `brew install ollama`, run `ollama serve` in a terminal pane, then re-run `plexi ai setup`.");
        return 0;
    }

    println!("{green}\u{2713}{reset} Ollama installed");

    // ── Step 3: check if Ollama is running ───────────────────────────────────
    let (ollama_running, ollama_models) = probe_ollama();
    log::info!(
        "cli:ai:setup: ollama_running={ollama_running} models_count={}",
        ollama_models.len()
    );

    if !ollama_running {
        println!();
        println!("{bold}Step 2/3: Start Ollama{reset}");
        println!("  Ollama is installed but not running.");
        println!();
        println!("  Start it in another terminal pane:");
        println!("    {dim}ollama serve{reset}");
        println!();
        println!("{yellow}Re-run `plexi ai setup` after starting Ollama.{reset}");
        crate::cli::print_tip("Open a new Plexi pane with Cmd+D, run `ollama serve`, then come back and re-run `plexi ai setup`.");
        return 0;
    }

    println!("{green}\u{2713}{reset} Ollama is running");

    // ── Step 4: check if recommended model is already pulled ─────────────────
    let already_have_model = ollama_models
        .iter()
        .any(|m| m == recommended_model || m.starts_with(&format!("{recommended_model}:")));

    log::info!(
        "cli:ai:setup: already_have_model={already_have_model} models={:?}",
        ollama_models
    );

    if already_have_model {
        println!("{green}\u{2713}{reset} Model {recommended_model} is already pulled");
    } else {
        println!();
        println!("{bold}Step 3/3: Pull recommended model{reset}");
        println!("  Pulling {bold}{recommended_model}{reset}...");
        println!("  {dim}(this may take a few minutes on first run){reset}");
        println!();

        log::info!("cli:ai:setup: running `ollama pull {recommended_model}`");

        let status = std::process::Command::new("ollama")
            .args(["pull", recommended_model])
            .status();

        match status {
            Ok(s) if s.success() => {
                log::info!("cli:ai:setup: ollama pull succeeded");
                println!("{green}\u{2713}{reset} Model {recommended_model} pulled successfully");
            }
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                log::warn!("cli:ai:setup: ollama pull failed with exit code {code}");
                eprintln!(
                    "{red}error:{reset} `ollama pull {recommended_model}` exited with code {code}"
                );
                eprintln!("Run it manually and then re-run `plexi ai setup`.");
                return 1;
            }
            Err(e) => {
                log::warn!("cli:ai:setup: could not spawn ollama pull: {e}");
                eprintln!("{red}error:{reset} could not run `ollama pull`: {e}");
                eprintln!("Ensure `ollama` is in your PATH and try again.");
                return 1;
            }
        }
    }

    // ── Step 5: write config.toml ─────────────────────────────────────────────
    let config_path = crate::config::config_path();
    log::info!(
        "cli:ai:setup: writing ollama config to {}",
        config_path.display()
    );

    match write_ollama_config(recommended_model) {
        Ok(()) => {
            println!(
                "{green}\u{2713}{reset} Config updated: {}",
                config_path.display()
            );
        }
        Err(e) => {
            log::warn!("cli:ai:setup: failed to write config: {e}");
            eprintln!(
                "{red}error:{reset} could not write config to {}: {e}",
                config_path.display()
            );
            eprintln!(
                "You can configure it manually — add this to {}:",
                config_path.display()
            );
            eprintln!();
            eprintln!("[ai]");
            eprintln!("backend = \"ollama\"");
            eprintln!();
            eprintln!("[ai.ollama]");
            eprintln!("host         = \"http://localhost:11434\"");
            eprintln!("model_low    = \"{recommended_model}\"");
            eprintln!("model_medium = \"{recommended_model}\"");
            eprintln!("model_high   = \"{recommended_model}\"");
            return 1;
        }
    }

    println!();
    println!("{bold}Setup complete!{reset}");
    println!("  Plexi apps using the `ai.query` capability will now use {green}{recommended_model}{reset} via Ollama.");
    println!();
    println!("Next steps:");
    println!("  {dim}plexi ai doctor{reset}    — verify your full AI configuration");
    println!("  {dim}plexi config edit{reset}  — tune model tiers or spending caps");

    crate::cli::print_tip(
        "Use `plexi ai doctor` at any time to see your hardware and integration status.",
    );

    0
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn ai_doctor_cli(json: bool) -> i32 {
    log::info!("cli:ai:doctor: starting hardware scan (json={json})");

    let hw = detect_hardware();
    log::info!(
        "cli:ai:doctor: hardware detected -- cpu={:?} ram_gb={:?} is_apple_silicon={} vram_gb={:?}",
        hw.cpu_name,
        hw.ram_gb,
        hw.is_apple_silicon,
        hw.vram_gb
    );

    let integrations = check_integrations();
    log::info!(
        "cli:ai:doctor: integrations -- ollama_installed={} ollama_running={} models={} openrouter={}",
        integrations.ollama_installed,
        integrations.ollama_running,
        integrations.ollama_models.len(),
        integrations.openrouter_configured
    );

    let recommendation = recommend_models(&hw);
    log::info!(
        "cli:ai:doctor: recommendation -- tier={} models={:?}",
        recommendation.tier,
        recommendation.models
    );

    if json {
        let report = AiDoctorReport {
            hardware: hw,
            integrations,
            recommendation,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: failed to serialize ai doctor report: {e}");
                return 1;
            }
        }
    } else {
        print_report(&hw, &integrations, &recommendation);
        crate::cli::print_tip("Run `plexi secret set openrouter-api-key --global` to configure cloud AI, or `brew install ollama && ollama pull llama3.2:3b` for local AI.");
    }

    0
}
