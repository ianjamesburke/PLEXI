use pty_process::{open, Command, OwnedReadPty, OwnedWritePty, Size};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;

pub struct PtySession {
    pub reader: OwnedReadPty,
    pub writer: OwnedWritePty,
    pub child: Child,
    pub cwd: String,
}

impl PtySession {
    pub fn spawn_shell(
        shell_path: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let resolved_cwd = resolve_working_dir(cwd);
        let (pty, pts) = open()?;
        pty.resize(Size::new(rows, cols))?;

        let path = std::env::var("PATH").unwrap_or_default();
        let extra = "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin";
        let full_path = if path.contains("/opt/homebrew/bin") {
            path
        } else {
            format!("{extra}:{path}")
        };

        let child = Command::new(shell_path)
            .current_dir(&resolved_cwd)
            .kill_on_drop(true)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("PATH", full_path)
            .spawn(pts)?;

        let (reader, writer) = pty.into_split();

        Ok(Self {
            reader,
            writer,
            child,
            cwd: resolved_cwd.to_string_lossy().into_owned(),
        })
    }
}

pub async fn write_input(
    writer: &mut OwnedWritePty,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

pub fn resize(
    writer: &OwnedWritePty,
    cols: u16,
    rows: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    writer.resize(Size::new(rows, cols))?;
    Ok(())
}

fn resolve_working_dir(cwd: Option<&str>) -> PathBuf {
    if let Some(cwd) = cwd {
        let cwd_path = Path::new(cwd);
        if cwd_path.exists() {
            return cwd_path.to_path_buf();
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(home);
        if home_path.exists() {
            return home_path;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}
