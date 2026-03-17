/// Real PTY spawning using pty-process crate
use std::io::{Read, Write};
use std::process::Child;
use pty_process::blocking::{Pty, Command};

pub struct PtyManager {
    /// The PTY master side
    pty: Option<Pty>,
    /// The child process
    child: Option<Child>,
}

impl PtyManager {
    pub fn new() -> Self {
        PtyManager {
            pty: None,
            child: None,
        }
    }

    /// Spawn a new PTY with the given shell
    pub fn spawn_shell(
        &mut self,
        shell_path: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Create a new PTY
        let mut pty = Pty::new()?;
        
        // Resize to requested dimensions
        pty.resize(pty_process::Size::new(rows, cols))?;

        // Create command
        let mut cmd = Command::new(shell_path);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }

        // Spawn the child process with the PTY
        let child = cmd.spawn(&pty.pts()?)?;

        self.pty = Some(pty);
        self.child = Some(child);
        
        log::info!("Spawned shell: {} ({}x{})", shell_path, cols, rows);
        Ok(())
    }

    /// Read available output from PTY
    pub fn read_output(&mut self, buf: &mut [u8]) -> Result<usize, Box<dyn std::error::Error>> {
        if let Some(ref mut pty) = self.pty {
            match pty.read(buf) {
                Ok(n) => Ok(n),
                Err(e) => Err(Box::new(e)),
            }
        } else {
            Err("No active PTY".into())
        }
    }

    /// Write input to PTY
    pub fn write_input(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut pty) = self.pty {
            pty.write_all(data)?;
            pty.flush()?;
            Ok(())
        } else {
            Err("No active PTY".into())
        }
    }

    /// Resize PTY window
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut pty) = self.pty {
            pty.resize(pty_process::Size::new(rows, cols))?;
            Ok(())
        } else {
            Err("No active PTY".into())
        }
    }

    /// Check if PTY is still alive
    pub fn is_alive(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_status)) => Ok(false),
                Ok(None) => Ok(true),
                Err(e) => Err(Box::new(e)),
            }
        } else {
            Ok(false)
        }
    }

    /// Close and cleanup PTY
    pub fn close(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut child) = self.child {
            // Try to kill gracefully
            let _ = child.kill();
            let _ = child.wait();
        }
        self.pty = None;
        self.child = None;
        Ok(())
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Ignore by default, run with --ignored flag
    fn test_spawn_and_pwd() {
        let mut pty = PtyManager::new();
        
        // Spawn a shell
        let shell = if std::path::Path::new("/bin/zsh").exists() {
            "/bin/zsh"
        } else {
            "/bin/bash"
        };

        let result = pty.spawn_shell(shell, None, 80, 24);
        assert!(result.is_ok(), "Failed to spawn shell: {:?}", result);

        // Write pwd command
        let write_result = pty.write_input(b"pwd\n");
        assert!(write_result.is_ok(), "Failed to write command: {:?}", write_result);

        // Read output
        let mut buf = vec![0u8; 4096];
        let mut found_output = false;
        
        for i in 0..20 {
            match pty.read_output(&mut buf) {
                Ok(n) if n > 0 => {
                    let output = String::from_utf8_lossy(&buf[..n]);
                    println!("[Read {}] PTY output ({}b): {}", i, n, output);
                    
                    // Check if we got pwd output (should contain a path)
                    if output.contains("/") || output.contains("home") {
                        found_output = true;
                        break;
                    }
                }
                Ok(_) => {
                    println!("[Read {}] Empty read", i);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    println!("[Read {}] Error: {}", i, e);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }

        assert!(found_output, "Did not receive pwd output from PTY");
        let _ = pty.close();
    }
}
