use super::run::{ROUTINES_FILE, RoutinesCliConfig};

pub fn routine_list() -> i32 {
    log::info!("cli: routine list");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: could not determine current directory: {e}"); return 1; }
    };
    let config_path = cwd.join(ROUTINES_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No routines configured.");
            println!();
            println!("To set up routines, create {} in your project:", ROUTINES_FILE);
            println!("  [[routine]]");
            println!("  name = \"morning-sync\"");
            println!("  command = \"./scripts/morning.sh\"");
            println!("  schedule = \"weekdays at 9am\"");
            println!("  context = \"work\"");
            return 0;
        }
        Err(e) => { eprintln!("error: could not read {}: {e}", config_path.display()); return 1; }
    };
    let config: RoutinesCliConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: failed to parse {ROUTINES_FILE}: {e}"); return 1; }
    };
    if config.routine.is_empty() {
        println!("No routines defined in {ROUTINES_FILE}.");
        return 0;
    }
    println!("Routines:");
    for r in &config.routine {
        let next = match crate::scheduler::parse_schedule(&r.schedule) {
            Some(s) => crate::scheduler::next_fire_description(&s, None),
            None => "invalid schedule".to_string(),
        };
        let ctx_label = if r.context.is_empty() { "(active context)".to_string() } else { r.context.clone() };
        let ephemeral_label = if r.ephemeral { " [ephemeral]" } else { "" };
        println!("  {:20} {:<30} next: {}  context: {}{}",
            r.name, r.schedule, next, ctx_label, ephemeral_label);
    }
    0
}

/// `plexi routine run <name>` — manually fire a routine
pub fn routine_run(name: &str) -> i32 {
    log::info!("cli: routine run '{name}'");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: could not determine current directory: {e}"); return 1; }
    };
    let config_path = cwd.join(ROUTINES_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: no {ROUTINES_FILE} found in {}", cwd.display());
            return 1;
        }
        Err(e) => { eprintln!("error: could not read {}: {e}", config_path.display()); return 1; }
    };
    let config: RoutinesCliConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: failed to parse {ROUTINES_FILE}: {e}"); return 1; }
    };
    let routine = match config.routine.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => {
            eprintln!("error: routine '{name}' not found in {ROUTINES_FILE}");
            if !config.routine.is_empty() {
                let names: Vec<&str> = config.routine.iter().map(|r| r.name.as_str()).collect();
                eprintln!("Available routines: {}", names.join(", "));
            }
            return 1;
        }
    };

    // Spawn via spawn-queue as a terminal pane
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let payload = serde_json::json!({
        "type_id": "terminal",
        "args": [routine.command.clone()],
        "ephemeral": routine.ephemeral,
        "no_focus": false,
    });
    let file = queue_dir.join(format!("{ts}.json"));
    if let Err(e) = std::fs::write(&file, payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    println!("queued: run routine '{name}' — command: {}", routine.command);
    0
}
