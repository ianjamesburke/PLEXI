use std::path::Path;

pub fn detect_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if Path::new(&shell).exists() {
            return shell;
        }
    }

    for shell in [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
    ] {
        if Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    "/bin/sh".to_string()
}
