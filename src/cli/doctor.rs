use serde::Serialize;

#[derive(Serialize)]
struct DoctorReport {
    healthy: bool,
    apps: Vec<AppReport>,
}

#[derive(Serialize)]
struct AppReport {
    id: String,
    missing: Vec<String>,
}

pub fn doctor_cli(json: bool) -> i32 {
    log::info!("cli:doctor: starting capability audit (json={json})");

    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app_registry::AppRegistry::load(&cwd);
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );

    let installed = registry.list();
    if installed.is_empty() {
        if json {
            println!(r#"{{"healthy":true,"apps":[]}}"#);
        } else {
            println!("No apps installed.");
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
                println!("  {green}\u{2713}{reset} {ok_count} app(s) -- all capabilities satisfied");
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
    }

    log::info!("cli:doctor: audit complete -- {total} app(s), {sick_count} unhealthy");

    if healthy { 0 } else { 1 }
}
