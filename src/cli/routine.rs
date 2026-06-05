use super::open::pane_new_cli;
use super::run::{routines_file, RoutinesCliConfig};

pub fn routine_list() -> i32 {
    log::info!("cli: routine list");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: could not determine current directory: {e}"); return 1; }
    };
    let rf = routines_file();
    let config_path = cwd.join(&rf);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No routines configured.");
            println!();
            println!("To set up routines, create {} in your project:", rf);
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
        Err(e) => { eprintln!("error: failed to parse {rf}: {e}"); return 1; }
    };
    if config.routine.is_empty() {
        println!("No routines defined in {rf}.");
        return 0;
    }
    println!("Routines:");
    for r in &config.routine {
        let next = match crate::host::scheduler::parse_schedule(&r.schedule) {
            Some(s) => crate::host::scheduler::next_fire_description(&s, None),
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
    let rf = routines_file();
    let config_path = cwd.join(&rf);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: no {rf} found in {}", cwd.display());
            return 1;
        }
        Err(e) => { eprintln!("error: could not read {}: {e}", config_path.display()); return 1; }
    };
    let config: RoutinesCliConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: failed to parse {rf}: {e}"); return 1; }
    };
    let routine = match config.routine.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => {
            eprintln!("error: routine '{name}' not found in {rf}");
            if !config.routine.is_empty() {
                let names: Vec<&str> = config.routine.iter().map(|r| r.name.as_str()).collect();
                eprintln!("Available routines: {}", names.join(", "));
            }
            return 1;
        }
    };

    // Spawn via socket (when inside a Plexi pane) with spawn-queue fallback.
    // pane_new_cli implements the socket-first pattern used by all other spawn paths.
    log::info!("cli: routine run '{name}' — dispatching command: {}", routine.command);
    pane_new_cli(
        Some(&routine.command),
        Some(name),
        "split_h",
        None,
        None,
        routine.ephemeral,
        false,
        None,
        &[],
        None,
        &[],
    )
}
