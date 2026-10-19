//! Process-owned admission. The table has no I/O; the service samples off-paint.

use std::collections::{HashMap, VecDeque};

const SWAP_GROWTH_LIMIT: u64 = 256 * 1024 * 1024;
const BUILD_PROCESS_LIMIT: usize = 4;
const PRESSURE_WINDOW_SECS: u64 = 60;

#[derive(Default)]
struct Sample {
    swap_bytes: u64,
    build_processes: usize,
    // PID plus process start time: a reused PID is a different holder.
    owners: HashMap<u32, u64>,
}

struct Pool {
    max: u32,
    holders: HashMap<u32, u64>,
}

#[derive(Default)]
struct LeaseTable {
    pools: HashMap<String, Pool>,
    sample: Sample,
    swap_history: VecDeque<(u64, u64)>,
    swap_growth_bytes: u64,
}

impl LeaseTable {
    fn observe(&mut self, sample: Sample, seconds: u64) {
        self.swap_history
            .retain(|(time, _)| seconds.saturating_sub(*time) <= PRESSURE_WINDOW_SECS);
        self.swap_history.push_back((seconds, sample.swap_bytes));
        let baseline = self
            .swap_history
            .iter()
            .map(|(_, bytes)| *bytes)
            .min()
            .unwrap_or(sample.swap_bytes);
        self.swap_growth_bytes = sample.swap_bytes.saturating_sub(baseline);
        self.pools.retain(|name, pool| {
            pool.holders.retain(|pid, started| {
                let live = sample.owners.get(pid) == Some(started);
                if !live {
                    log::info!("lease: release name={name} holder_pid={pid} reason=process_exit");
                }
                live
            });
            !pool.holders.is_empty()
        });
        self.sample = sample;
    }

    fn acquire(&mut self, name: &str, max: u32, pid: u32) -> Value {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || max == 0
        {
            return error(
                "invalid_lease",
                "name must be 1-128 ASCII letters/digits/._- and max must be positive",
            );
        }
        let Some(&started) = self.sample.owners.get(&pid) else {
            return error("holder_exited", "holder process is no longer alive");
        };
        if let Some(pool) = self.pools.get(name) {
            if pool.max != max {
                return error("lease_max_mismatch", "pool already has a different maximum");
            }
            if pool.holders.get(&pid) == Some(&started) {
                return json!({"status": "acquired", "name": name, "holder_pid": pid, "max": max, "already_held": true});
            }
            if pool.holders.len() >= max as usize {
                log::info!("lease: admission=paused reason=lease_capacity name={name} max={max}");
                return json!({"status": "paused", "error_code": "lease_capacity", "name": name, "max": max});
            }
        }
        let pressured = self.swap_growth_bytes >= SWAP_GROWTH_LIMIT
            || self.sample.build_processes >= BUILD_PROCESS_LIMIT;
        log::info!("lease: machine_pressure admission={} name={name} swap_bytes={} swap_growth_bytes={} build_processes={}",
            if pressured { "paused" } else { "admitted" }, self.sample.swap_bytes, self.swap_growth_bytes, self.sample.build_processes);
        if pressured {
            return json!({"status": "paused", "error_code": "machine_pressure",
                "swap_bytes": self.sample.swap_bytes, "swap_growth_bytes": self.swap_growth_bytes,
                "build_processes": self.sample.build_processes});
        }
        self.pools
            .entry(name.to_owned())
            .or_insert_with(|| Pool {
                max,
                holders: HashMap::new(),
            })
            .holders
            .insert(pid, started);
        log::info!("lease: acquire name={name} holder_pid={pid} start_time={started} max={max}");
        json!({"status": "acquired", "name": name, "holder_pid": pid, "max": max, "already_held": false})
    }

    fn release(&mut self, name: Option<&str>, pid: u32) -> Value {
        let mut released = 0;
        self.pools.retain(|pool_name, pool| {
            if name.is_none_or(|name| name == pool_name) && pool.holders.remove(&pid).is_some() {
                released += 1;
                log::info!("lease: release name={pool_name} holder_pid={pid} reason=explicit");
            }
            !pool.holders.is_empty()
        });
        json!({"status": "released", "released": released, "holder_pid": pid})
    }
}

fn error(code: &str, message: &str) -> Value {
    json!({"error_code": code, "error": message})
}

pub(crate) struct LeaseRequest {
    pub op: String,
    pub name: Option<String>,
    pub max: Option<u32>,
    pub holder_pid: u32,
    pub response_file: String,
}

/// Bounded mailbox, one owner thread, no host-paint dependency. Dropping the host
/// drops the sender and ends the sampler. Admission never retains a waiting job.
pub(crate) struct LeaseService {
    sender: std::sync::mpsc::SyncSender<LeaseRequest>,
}

impl LeaseService {
    pub(crate) fn start() -> std::io::Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<LeaseRequest>(16);
        std::thread::Builder::new()
            .name("resource-leases".into())
            .spawn(move || {
                let mut table = LeaseTable::default();
                let mut system = sysinfo::System::new();
                let started = std::time::Instant::now();
                loop {
                    let request = match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                        Ok(request) => Some(request),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    table.observe(sample_machine(&mut system), started.elapsed().as_secs());
                    if let Some(request) = request {
                        let result = match request.op.as_str() {
                            "acquire" => table.acquire(
                                request.name.as_deref().unwrap_or(""),
                                request.max.unwrap_or(0),
                                request.holder_pid,
                            ),
                            "release" => table.release(request.name.as_deref(), request.holder_pid),
                            _ => error("invalid_lease", "unknown lease operation"),
                        };
                        if !crate::rpc::write_json_response(&request.response_file, result.clone())
                            && result["status"] == "acquired"
                            && result["already_held"] == false
                        {
                            table.release(request.name.as_deref(), request.holder_pid);
                        }
                    }
                }
            })?;
        Ok(Self { sender })
    }

    pub(crate) fn submit(&self, request: LeaseRequest) {
        if let Err(error) = self.sender.try_send(request) {
            let request = match error {
                std::sync::mpsc::TrySendError::Full(request)
                | std::sync::mpsc::TrySendError::Disconnected(request) => request,
            };
            log::warn!("lease: admission=paused reason=service_busy");
            crate::rpc::write_json_response(
                &request.response_file,
                json!({"status": "paused", "error_code": "lease_service_busy"}),
            );
        }
    }
}

fn sample_machine(system: &mut sysinfo::System) -> Sample {
    // On macOS total_swap is vm.swapusage.xsu_total: allocated swap-file bytes,
    // not free RAM (nor a percentage of RAM). Only request swap refresh.
    system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::new().with_swap());
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    let mut sample = Sample {
        swap_bytes: system.total_swap(),
        ..Sample::default()
    };
    for (pid, process) in system.processes() {
        if matches!(
            process.status(),
            sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
        ) {
            continue;
        }
        sample.owners.insert(pid.as_u32(), process.start_time());
        if matches!(
            process.name().to_str(),
            Some("cargo" | "rustc" | "cargo.exe" | "rustc.exe")
        ) {
            sample.build_processes += 1;
        }
    }
    sample
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(swap: u64, builds: usize, owners: &[(u32, u64)]) -> Sample {
        Sample {
            swap_bytes: swap,
            build_processes: builds,
            owners: owners.iter().copied().collect(),
        }
    }

    #[test]
    fn leases_cap_release_death_and_pid_reuse() {
        let mut table = LeaseTable::default();
        table.observe(sample(0, 0, &[(10, 1), (20, 2)]), 0);
        assert_eq!(table.acquire("lanes", 1, 10)["status"], "acquired");
        assert_eq!(
            table.acquire("lanes", 1, 20)["error_code"],
            "lease_capacity"
        );
        assert_eq!(
            table.acquire("lanes", 2, 20)["error_code"],
            "lease_max_mismatch"
        );
        assert_eq!(table.release(Some("lanes"), 10)["released"], 1);
        assert_eq!(table.acquire("lanes", 1, 20)["status"], "acquired");
        table.observe(sample(0, 0, &[(10, 1), (20, 3)]), 1);
        assert!(
            table.pools.is_empty(),
            "PID reuse must reclaim the old lifetime"
        );
        assert_eq!(table.acquire("lanes", 1, 10)["status"], "acquired");
        table.observe(sample(0, 0, &[]), 2);
        assert!(table.pools.is_empty());
    }

    #[test]
    fn leases_pressure_swap_growth_and_build_count_pause_without_queueing() {
        let mut table = LeaseTable::default();
        table.observe(sample(13 << 30, 0, &[(10, 1)]), 0);
        table.observe(sample((13 << 30) + SWAP_GROWTH_LIMIT, 0, &[(10, 1)]), 1);
        assert_eq!(
            table.acquire("lanes", 1, 10)["error_code"],
            "machine_pressure"
        );
        assert!(table.pools.is_empty());
        table.observe(sample(14 << 30, 0, &[(10, 1)]), 62);
        assert_eq!(table.acquire("lanes", 1, 10)["status"], "acquired");
        table.release(None, 10);
        table.observe(sample(14 << 30, BUILD_PROCESS_LIMIT, &[(10, 1)]), 63);
        assert_eq!(table.acquire("cargo-build", 1, 10)["status"], "paused");
        assert!(table.pools.is_empty());
    }

    #[test]
    fn leases_retries_are_idempotent_and_release_is_owner_scoped() {
        let mut table = LeaseTable::default();
        table.observe(sample(0, 0, &[(10, 1), (20, 2)]), 0);
        assert_eq!(table.acquire("cargo-build", 1, 10)["status"], "acquired");
        assert_eq!(table.acquire("cargo-build", 1, 10)["status"], "acquired");
        assert_eq!(table.release(None, 20)["released"], 0);
        assert_eq!(
            table.acquire("cargo-build", 1, 20)["error_code"],
            "lease_capacity"
        );
        assert_eq!(
            table.acquire("invalid", 0, 10)["error_code"],
            "invalid_lease"
        );
        assert_eq!(table.acquire("valid", 1, 99)["error_code"], "holder_exited");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn leases_hidden_host_dispatch_and_owner_identity() {
        let mut harness = crate::testing::HostHarness::new();
        let dir = tempfile::tempdir().unwrap();
        let reply = dir.path().join("reply.json");
        harness.inject_ipc(crate::protocol::AppRequest::ResourceLease {
            op: "acquire".into(),
            name: Some("lanes".into()),
            max: Some(1),
            holder_pid: std::process::id(),
            peer_ancestry: vec![],
            response_file: reply.to_string_lossy().into_owned(),
        });
        harness.hidden_frame();
        let body: Value = serde_json::from_slice(&std::fs::read(&reply).unwrap()).unwrap();
        assert_eq!(body["error_code"], "permission_denied");
        harness.inject_ipc(crate::protocol::AppRequest::ResourceLease {
            op: "release".into(),
            name: None,
            max: None,
            holder_pid: std::process::id(),
            peer_ancestry: vec![std::process::id()],
            response_file: reply.to_string_lossy().into_owned(),
        });
        std::fs::remove_file(&reply).unwrap();
        harness.hidden_frame();
        let timeout = crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        let start = std::time::Instant::now();
        while !reply.exists() {
            assert!(start.elapsed() < timeout, "off-paint lease reply timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let body: Value = serde_json::from_slice(&std::fs::read(&reply).unwrap()).unwrap();
        assert_eq!(body["status"], "released");
    }

    #[test]
    #[cfg(unix)]
    fn leases_sampler_detects_real_process_exit() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut system = sysinfo::System::new();
        let mut sample = sample_machine(&mut system);
        assert!(sample.owners.contains_key(&child.id()));
        // Deterministic pressure input; use the real sampled process identities.
        sample.swap_bytes = 0;
        sample.build_processes = 0;
        let mut table = LeaseTable::default();
        table.observe(sample, 0);
        assert_eq!(table.acquire("lanes", 1, child.id())["status"], "acquired");
        child.kill().unwrap();
        child.wait().unwrap();
        table.observe(sample_machine(&mut system), 1);
        assert!(table.pools.is_empty());
    }
}
use serde_json::{json, Value};
