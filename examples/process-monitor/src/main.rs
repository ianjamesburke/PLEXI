//! process-monitor — Example Plexi app (Rust)
//!
//! Browse running processes. j/k navigate, Enter sends `kill <pid>`.
//!
//! Build:  cargo build --release
//! Install: cp target/release/plexi-app ~/.plexi/apps/process-monitor/bin/plexi-app
//!          cp manifest.toml ~/.plexi/apps/process-monitor/

use plexi_sdk::{App, Emitter, Modifiers, RenderContext, run};
use std::process::Command;

const BG: &str = "#1e1e2e";
const HEADER: &str = "#181825";
const TEXT: &str = "#cdd6f4";
const MUTED: &str = "#6c7086";
const ACCENT: &str = "#89b4fa";
const SEL_BG: &str = "#313244";

const HEADER_H: f32 = 44.0;
const ITEM_H: f32 = 44.0;

struct ProcessMonitor {
    procs: Vec<ProcessEntry>,
    selected: usize,
    scroll_offset: usize,
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
        let mut m = Self { procs: Vec::new(), selected: 0, scroll_offset: 0 };
        m.refresh();
        m
    }

    fn refresh(&mut self) {
        self.procs = load_processes();
        self.selected = self.selected.min(self.procs.len().saturating_sub(1));
    }
}

fn load_processes() -> Vec<ProcessEntry> {
    let output = match Command::new("ps").args(["aux"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries: Vec<ProcessEntry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        // Skip the header line (USER PID %CPU ...)
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
        // Background
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, BG);

        let visible_rows = ((ctx.height - HEADER_H) / ITEM_H).max(1.0) as usize;

        // Clamp scroll so selected row is always visible
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected - visible_rows + 1;
        }

        // Render process rows BEFORE header so header paints on top
        if self.procs.is_empty() {
            ctx.text(16.0, HEADER_H + 14.0, "No processes found.", 13.0, MUTED);
        } else {
            for (i, proc) in self.procs.iter().enumerate() {
                if i < self.scroll_offset { continue; }
                let row_index = i - self.scroll_offset;
                if row_index >= visible_rows { break; }

                let y = HEADER_H + row_index as f32 * ITEM_H;
                let is_sel = i == self.selected;

                if is_sel {
                    ctx.rect(0.0, y, ctx.width, ITEM_H, SEL_BG);
                }

                let text_color = if is_sel { TEXT } else { MUTED };
                let cmd = truncate(&proc.command, 50);
                ctx.text(16.0, y + 10.0, &cmd, 12.0, text_color);
                let detail = format!("PID {}  CPU {}%  MEM {}%  {}", proc.pid, proc.cpu, proc.mem, proc.user);
                ctx.text(16.0, y + 26.0, &detail, 10.0, MUTED);
            }
        }

        // Header — drawn AFTER rows so it always paints on top
        ctx.rect(0.0, 0.0, ctx.width, HEADER_H, HEADER);
        ctx.text_bold(16.0, 13.0, "Process Monitor", 14.0, ACCENT);
        let hint = "j/k navigate  Enter=kill  r=refresh";
        ctx.text(ctx.width - (hint.len() as f32 * 7.2), 13.0, hint, 11.0, MUTED);
    }

    fn on_key(&mut self, key: &str, _mods: &Modifiers, emit: &mut Emitter) {
        // Plexi sends printable characters via Event::Text as lowercase (e.g. "j", "r").
        // Control/navigation keys come via Event::Key as PascalCase (e.g. "ArrowDown", "Enter").
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
