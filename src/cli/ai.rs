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
    let cpu_cores = run_sysctl("hw.ncpu")
        .and_then(|s| s.trim().parse::<u32>().ok());
    let ram_bytes = run_sysctl("hw.memsize")
        .and_then(|s| s.trim().parse::<u64>().ok());
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
        if s.is_empty() { None } else { Some(s) }
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
    val["SPDisplaysDataType"]
        .as_array()?
        .first()?["sppci_model"]
        .as_str()
        .map(|s| s.to_string())
}

/// Parse discrete VRAM from system_profiler JSON.
/// Field is "spdisplays_vram" — value like "8192 MB" or "16 GB".
fn parse_vram_from_profiler(json: &str) -> Option<f64> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;
    let vram_str = val["SPDisplaysDataType"]
        .as_array()?
        .first()?["spdisplays_vram"]
        .as_str()?;

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
        println!("  RAM:       {:.1} GB{}", ram, if hw.is_apple_silicon { " (unified)" } else { "" });
    }
    if let Some(ref gpu) = hw.gpu_name {
        println!("  GPU:       {gpu}");
    }
    if let Some(vram) = hw.vram_gb {
        let label = if hw.is_apple_silicon { "VRAM (unified):" } else { "VRAM:          " };
        println!("  {label} {vram:.1} GB");
    }
    if let Some(disk) = hw.disk_free_gb {
        let disk_color = if disk >= 20.0 { green } else if disk >= 5.0 { yellow } else { red };
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
