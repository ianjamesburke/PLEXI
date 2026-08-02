//! Host-owned named build leases.
//!
//! A lease is held by a pane, rather than by a shell PID: the host can prove
//! and clean up that identity when the terminal exits or is closed.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use serde::Deserialize;

#[derive(Debug)]
struct Holder {
    pane_id: u64,
}

#[derive(Debug)]
struct Waiter {
    pane_id: u64,
    response_file: String,
    queued_at: Instant,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct NamedLease {
    holders: Vec<Holder>,
    waiters: VecDeque<Waiter>,
}

#[derive(Debug)]
pub(crate) struct BuildLeaseManager {
    capacity: usize,
    leases: HashMap<String, NamedLease>,
}

#[derive(Deserialize)]
struct RunConfig {
    limits: RunLimits,
}

#[derive(Deserialize)]
struct RunLimits {
    max_concurrent_cargo_consumers: usize,
}

impl BuildLeaseManager {
    pub(crate) fn from_run_config() -> Self {
        let config: RunConfig = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.agents/skills/babysitter/RUN_CONFIG.toml"
        )))
        .expect("RUN_CONFIG.toml must define limits.max_concurrent_cargo_consumers");
        Self::new(config.limits.max_concurrent_cargo_consumers)
    }

    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "build lease capacity must be positive");
        Self {
            capacity,
            leases: HashMap::new(),
        }
    }

    pub(crate) fn acquire(
        &mut self,
        name: &str,
        pane_id: u64,
        timeout_secs: f64,
        response_file: String,
    ) -> Result<(), String> {
        if !timeout_secs.is_finite()
            || !(0.0..=crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS).contains(&timeout_secs)
        {
            return Err(format!(
                "--timeout must be between 0 and {}, got {timeout_secs}",
                crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
            ));
        }
        let now = Instant::now();
        let lease = self.leases.entry(name.to_owned()).or_default();
        if lease.holders.iter().any(|holder| holder.pane_id == pane_id)
            || lease.waiters.iter().any(|waiter| waiter.pane_id == pane_id)
        {
            return Err(format!(
                "pane {pane_id} already owns or waits for lease {name:?}; leases are non-reentrant"
            ));
        }
        if lease.waiters.is_empty() && lease.holders.len() < self.capacity {
            if !crate::rpc::write_json_response(
                &response_file,
                serde_json::json!({"ok": true, "name": name, "pane_id": pane_id, "wait_secs": 0.0}),
            ) {
                return Err(format!(
                    "could not confirm lease {name:?} grant to pane {pane_id}"
                ));
            }
            lease.holders.push(Holder { pane_id });
            log::info!("build_lease: granted name={name:?} pane_id={pane_id} wait_secs=0");
        } else {
            lease.waiters.push_back(Waiter {
                pane_id,
                response_file,
                queued_at: now,
                expires_at: now + std::time::Duration::from_secs_f64(timeout_secs),
            });
            log::info!(
                "build_lease: queued name={name:?} pane_id={pane_id} depth={}",
                lease.waiters.len()
            );
        }
        Ok(())
    }

    pub(crate) fn release(&mut self, name: &str, pane_id: u64) -> Result<(), String> {
        let Some(lease) = self.leases.get_mut(name) else {
            return Err(format!("lease {name:?} is not held"));
        };
        let before = lease.holders.len();
        lease.holders.retain(|holder| holder.pane_id != pane_id);
        if before == lease.holders.len() {
            return Err(format!("pane {pane_id} does not hold lease {name:?}"));
        }
        log::info!("build_lease: released name={name:?} pane_id={pane_id}");
        self.grant_waiters(name);
        Ok(())
    }

    pub(crate) fn release_pane(&mut self, pane_id: u64, reason: &str) {
        let names: Vec<String> = self.leases.keys().cloned().collect();
        for name in names {
            let Some(lease) = self.leases.get_mut(&name) else {
                continue;
            };
            let held = lease.holders.iter().any(|holder| holder.pane_id == pane_id);
            lease.holders.retain(|holder| holder.pane_id != pane_id);
            lease.waiters.retain(|waiter| waiter.pane_id != pane_id);
            if held {
                log::info!(
                    "build_lease: auto_released name={name:?} pane_id={pane_id} reason={reason}"
                );
                self.grant_waiters(&name);
            }
        }
    }

    pub(crate) fn status(&self, name: &str) -> serde_json::Value {
        let now = Instant::now();
        let Some(lease) = self.leases.get(name) else {
            return serde_json::json!({"name": name, "holder_pane_id": null, "holder_pane_ids": [], "queue_depth": 0, "wait_secs": 0.0});
        };
        let holders: Vec<u64> = lease.holders.iter().map(|holder| holder.pane_id).collect();
        let wait_secs = lease
            .waiters
            .front()
            .map(|waiter| now.duration_since(waiter.queued_at).as_secs_f64())
            .unwrap_or(0.0);
        serde_json::json!({"name": name, "holder_pane_id": holders.first(), "holder_pane_ids": holders, "queue_depth": lease.waiters.len(), "wait_secs": wait_secs})
    }

    pub(crate) fn service(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let names: Vec<String> = self.leases.keys().cloned().collect();
        for name in &names {
            let mut expired = Vec::new();
            if let Some(lease) = self.leases.get_mut(name) {
                lease.waiters.retain(|waiter| {
                    if now >= waiter.expires_at {
                        expired.push((waiter.pane_id, waiter.response_file.clone()));
                        false
                    } else {
                        true
                    }
                });
            }
            for (pane_id, response_file) in expired {
                log::info!("build_lease: timed_out name={name:?} pane_id={pane_id}");
                crate::rpc::write_json_response(
                    &format!("{response_file}.err"),
                    serde_json::json!({"ok": false, "timeout": true, "error": format!("timed out waiting for lease {name:?}")}),
                );
            }
            self.grant_waiters(name);
        }
        if let Some(next) = self
            .leases
            .values()
            .flat_map(|lease| lease.waiters.iter().map(|waiter| waiter.expires_at))
            .min()
        {
            let remaining = next.saturating_duration_since(Instant::now());
            let predicted =
                std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
            ctx.request_repaint_after(remaining + predicted);
        }
    }

    fn grant_waiters(&mut self, name: &str) {
        let now = Instant::now();
        let Some(lease) = self.leases.get_mut(name) else {
            return;
        };
        while lease.holders.len() < self.capacity {
            let Some(waiter) = lease.waiters.pop_front() else {
                break;
            };
            if now >= waiter.expires_at {
                crate::rpc::write_json_response(
                    &format!("{}.err", waiter.response_file),
                    serde_json::json!({"ok": false, "timeout": true, "error": format!("timed out waiting for lease {name:?}")}),
                );
                continue;
            }
            let wait_secs = now.duration_since(waiter.queued_at).as_secs_f64();
            if !crate::rpc::write_json_response(
                &waiter.response_file,
                serde_json::json!({"ok": true, "name": name, "pane_id": waiter.pane_id, "wait_secs": wait_secs}),
            ) {
                log::error!(
                    "build_lease: could not confirm grant name={name:?} pane_id={}; dropping waiter",
                    waiter.pane_id
                );
                continue;
            }
            lease.holders.push(Holder {
                pane_id: waiter.pane_id,
            });
            log::info!(
                "build_lease: granted name={name:?} pane_id={} wait_secs={wait_secs:.3}",
                waiter.pane_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    #[test]
    fn cargo_lease_wrapper_explicitly_proceeds_without_a_host() {
        let output = Command::new("bash")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/cargo-with-lease.sh"
            ))
            .arg("/usr/bin/true")
            .env_remove("PLEXI_SOCKET")
            .env_remove("PLEXI_CHANNEL")
            .env_remove("PLEXI_PANE_ID")
            .output()
            .expect("cargo lease wrapper must run");
        assert!(
            output.status.success(),
            "unreachable host must not block cargo"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("proceeding unleased (no Plexi host reachable)"),
            "unreachable host fallback must be explicit"
        );
    }

    #[test]
    fn cargo_lease_wrapper_refuses_a_reachable_host_with_failed_status() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let plexi = bin.join("plexi");
        std::fs::write(
            &plexi,
            "#!/usr/bin/env bash\nif [[ \"$1\" == pane ]]; then exit 0; fi\nexit 2\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&plexi).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&plexi, permissions).unwrap();

        let output = Command::new("bash")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/cargo-with-lease.sh"
            ))
            .arg("/usr/bin/true")
            .env("PLEXI_SOCKET", "/tmp/reachable-plexi.sock")
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .expect("cargo lease wrapper must run");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("host reachable but status failed"),
            "a configured host that cannot answer status must not run cargo unleased"
        );
    }

    #[test]
    fn cargo_lease_wrapper_surfaces_release_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let plexi = bin.join("plexi");
        std::fs::write(
            &plexi,
            "#!/usr/bin/env bash\ncase \"$1 $2\" in\n  'pane list'|'lock status'|'lock acquire') exit 0 ;;\n  'lock release') exit 2 ;;\nesac\nexit 1\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&plexi).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&plexi, permissions).unwrap();

        let output = Command::new("bash")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/cargo-with-lease.sh"
            ))
            .arg("/usr/bin/true")
            .env("PLEXI_SOCKET", "/tmp/reachable-plexi.sock")
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .expect("cargo lease wrapper must run");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("release failed"),
            "a failed release must fail the build command rather than strand the holder"
        );
    }

    #[test]
    fn capacity_controls_parallel_holders() {
        let mut leases = BuildLeaseManager::new(1);
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.json");
        let second = dir.path().join("second.json");
        leases
            .acquire("cargo-build", 1, 60.0, first.to_string_lossy().into())
            .unwrap();
        leases
            .acquire("cargo-build", 2, 60.0, second.to_string_lossy().into())
            .unwrap();
        assert!(first.exists());
        assert!(
            !second.exists(),
            "capacity one must queue the second holder"
        );
        leases.release("cargo-build", 1).unwrap();
        assert!(second.exists(), "release must grant the FIFO waiter");
    }

    #[test]
    fn holder_exit_releases_lease_and_grants_the_fifo_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let mut leases = BuildLeaseManager::new(1);
        let granted = dir.path().join("holder.json");
        let queued = dir.path().join("waiter.json");
        leases
            .acquire("cargo-build", 1, 60.0, granted.to_string_lossy().into())
            .unwrap();
        assert!(granted.exists());
        leases
            .acquire("cargo-build", 2, 60.0, queued.to_string_lossy().into())
            .unwrap();
        assert!(!queued.exists(), "capacity one must queue the second pane");
        leases.release_pane(1, "pty_exit");
        assert!(
            queued.exists(),
            "PTY death must release the holder and grant FIFO waiter"
        );
    }

    #[test]
    fn saturated_host_returns_a_typed_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let mut leases = BuildLeaseManager::new(1);
        leases
            .acquire(
                "cargo-build",
                1,
                60.0,
                dir.path().join("holder.json").to_string_lossy().into(),
            )
            .unwrap();
        let timeout = dir.path().join("timeout.json");
        leases
            .acquire("cargo-build", 2, 0.0, timeout.to_string_lossy().into())
            .unwrap();
        leases.service(&egui::Context::default());
        let reply: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{}.err", timeout.display())).unwrap())
                .unwrap();
        assert_eq!(
            reply["timeout"], true,
            "reachable saturated host must fail rather than proceed unleased"
        );
    }

    #[test]
    fn failed_grant_response_does_not_create_a_holder() {
        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("missing/grant.json");
        let mut manager = BuildLeaseManager::new(1);

        assert!(manager
            .acquire(
                "cargo-build",
                7,
                1.0,
                response.to_string_lossy().into_owned()
            )
            .is_err());
        assert_eq!(
            manager.status("cargo-build")["holder_pane_id"],
            serde_json::Value::Null,
            "a client that never receives its grant must not occupy capacity"
        );
    }

    #[test]
    fn later_waiter_times_out_behind_a_live_fifo_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let mut leases = BuildLeaseManager::new(1);
        leases
            .acquire(
                "cargo-build",
                1,
                60.0,
                dir.path().join("holder.json").to_string_lossy().into(),
            )
            .unwrap();
        let live = dir.path().join("live.json");
        let expired = dir.path().join("expired.json");
        leases
            .acquire("cargo-build", 2, 60.0, live.to_string_lossy().into())
            .unwrap();
        leases
            .acquire("cargo-build", 3, 0.0, expired.to_string_lossy().into())
            .unwrap();
        leases.service(&egui::Context::default());
        assert!(
            std::path::Path::new(&format!("{}.err", expired.display())).exists(),
            "an expired waiter behind a live FIFO waiter must receive the host timeout"
        );
        assert_eq!(leases.status("cargo-build")["queue_depth"], 1);
    }
}
