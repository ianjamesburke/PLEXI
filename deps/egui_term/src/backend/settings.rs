use std::collections::HashMap;
use std::path::PathBuf;

#[cfg(unix)]
const DEFAULT_SHELL: &str = "/bin/bash";
// Windows default: cmd.exe always exists at the COMSPEC location. The host
// usually overrides this via `shell::detect_shell()` (which prefers
// pwsh.exe / Windows Terminal's profile defaults), so this is only a safety
// net for code paths that take `BackendSettings::default()` directly.
#[cfg(windows)]
const DEFAULT_SHELL: &str = "cmd.exe";

#[derive(Debug, Clone)]
pub struct BackendSettings {
    pub shell: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub dynamic_colors: HashMap<usize, [u8; 3]>,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            shell: DEFAULT_SHELL.to_string(),
            args: vec![],
            working_directory: None,
            env: HashMap::new(),
            dynamic_colors: HashMap::new(),
        }
    }
}
