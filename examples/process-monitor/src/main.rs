//! process-monitor — Example Plexi app (Rust)
//!
//! Browse running processes. j/k navigate, Enter sends `kill <pid>`.
//!
//! Build:  cargo build --release
//! Install: cp target/release/plexi-app ~/.plexi/apps/process-monitor/bin/plexi-app
//!          cp manifest.toml ~/.plexi/apps/process-monitor/

use plexi_sdk::{App, Emitter, ListItem, Modifiers, RenderContext, run};
use std::process::Command;

const BG: &str = "#1e1e2e";
const HEADER: &str = "#181825";
const TEXT: &str = "#cdd6f4";
const MUTED: &str = "#6c7086";
const ACCENT: &str = "#89b4fa";

struct ProcessMonitor {
    procs: Vec<ProcessEntry>,
    selected: usize,
}

#[derive(Clone)]
struct ProcessEntry {
    pid: String,
    user: String,
    cpu: String,
    mem: String,
    command: String,
}

impl ProcessMonitor {
    fn new() -> Self {
        let mut m = Self { procs: Vec::new(), selected: 0 };
        m.refresh();
        m
    }

    fn refresh(&mut self) {
        self.procs = load_processes();
        self.selected = self.selected.min(self.procs.len().saturating_sub(1));
    }
}

fn load_processes() -> Vec<ProcessEntry> {
    let output = Command::new("ps")
        .args(["aux", "--no-header"])
        .output()
        .or_else(|_| Command::new("ps").args(["aux"]).output());

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries: Vec<ProcessEntry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        // skip header line (USER PID ...)
        .filter(|l| !l.trim_start().starts_with("USER"))
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 11 { return None; }
            Some(ProcessEntry {
                user:    cols[0].to_string(),
                pid:     cols[1].to_string(),
                cpu:     cols[2].to_string(),
                mem:     cols[3].to_string(),
                command: cols[10..].join(" "),
            })
        })
        .collect();

    // Sort by CPU descending
    entries.sort_by(|a, b| {
        b.cpu.parse::<f32>().unwrap_or(0.0)
            .partial_cmp(&a.cpu.parse::<f32>().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    entries.truncate(50);
    entries
}

impl App for ProcessMonitor {
    fn on_render(&mut self, ctx: &mut RenderContext) {
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, BG);

        // Header
        ctx.rect(0.0, 0.0, ctx.width, 44.0, HEADER);
        ctx.text_bold(16.0, 13.0, "Process Monitor", 14.0, ACCENT);
        let hint = "j/k navigate  Enter=kill  r=refresh";
        ctx.text(ctx.width - (hint.len() as f32 * 7.2), 13.0, hint, 11.0, MUTED);

        if self.procs.is_empty() {
            ctx.text(16.0, 64.0, "No processes found.", 13.0, MUTED);
            ctx.list(vec![], 0, 40.0);
            return;
        }

        let items: Vec<ListItem> = self.procs.iter().map(|p| {
            ListItem::new(truncate(&p.command, 60))
                .secondary(format!("PID {}  CPU {}%  MEM {}%  {}", p.pid, p.cpu, p.mem, p.user))
        }).collect();

        ctx.list(items, self.selected, 48.0);
    }

    fn on_key(&mut self, key: &str, _mods: &Modifiers, emit: &mut Emitter) {
        match key {
            "j" | "ArrowDown" => {
                self.selected = (self.selected + 1).min(self.procs.len().saturating_sub(1));
            }
            "k" | "ArrowUp" => {
                self.selected = self.selected.saturating_sub(1);
            }
            "Enter" => {
                if let Some(p) = self.procs.get(self.selected) {
                    emit.run_in_terminal(&format!("kill {}", p.pid));
                }
            }
            "r" => self.refresh(),
            _ => {}
        }
    }

    fn on_command(&mut self, text: &str, emit: &mut Emitter) {
        let text = text.trim();
        if text == "r" || text == "refresh" {
            self.refresh();
        } else if let Some(pid) = text.strip_prefix("kill ") {
            emit.run_in_terminal(&format!("kill {}", pid.trim()));
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}…", &s[..max - 1]) }
}

fn main() {
    run(&mut ProcessMonitor::new());
}
