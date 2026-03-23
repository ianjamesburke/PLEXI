use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_SHELL: &str = "/bin/bash";

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
