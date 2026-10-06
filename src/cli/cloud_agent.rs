//! Local container runner for one packaged agent.
//!
//! Hosting rules live in `docs/security/cloud-hosting-guardrails.md`. This
//! module does not restate them. The runner never mounts the host home, the
//! host profile, or the docker socket, and it never approves a tool call.
//! Admission happens inside the tenant, through [`admit_house_tool`].

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, TargetType,
};
use crate::cloud::retention::{self, RetentionInput, RetentionReport};

const HOUSE_PACKAGE: &str = "house";
const HOUSE_ACTOR: &str = "chess-opponent";
const HOUSE_AGENT: &str = "chess-opponent";
const TENANT_UID: &str = "65532:65532";

struct ToolPolicy {
    tool: &'static str,
    decision: Decision,
}

const HOUSE_POLICIES: &[ToolPolicy] = &[
    ToolPolicy {
        tool: "agent.turn",
        decision: Decision::Allow,
    },
    ToolPolicy {
        tool: "app.chess.state",
        decision: Decision::Allow,
    },
    ToolPolicy {
        tool: "app.chess.legal_moves",
        decision: Decision::Allow,
    },
    ToolPolicy {
        tool: "app.chess.play",
        decision: Decision::Deny,
    },
    ToolPolicy {
        tool: "host.secret.read",
        decision: Decision::Deny,
    },
    ToolPolicy {
        tool: "model.complete",
        decision: Decision::Allow,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct HouseNames {
    network: String,
    edge: String,
    relay: String,
    agent: String,
    volume: String,
    relay_volume: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AdmitOutcome {
    pub decision: &'static str,
    pub code: &'static str,
    pub exit: i32,
}

struct StagedImage {
    context: PathBuf,
    tag: String,
}

impl Drop for StagedImage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.context);
    }
}

pub fn cloud_agent_run_cli(agent: Option<&str>, tenant: Option<&str>) -> i32 {
    match run_house(agent, tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_stop_cli(tenant: Option<&str>) -> i32 {
    match stop_house(tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_status_cli(tenant: Option<&str>) -> i32 {
    match status_house(tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_admit_cli(
    profile: &Path,
    workspace: &Path,
    tool: &str,
    input: &str,
    actor: Option<&str>,
) -> i32 {
    match admit_house_tool(profile, workspace, tool, input, actor) {
        Ok(outcome) => {
            let value = serde_json::json!({
                "decision": outcome.decision,
                "code": outcome.code,
                "tool": tool,
            });
            let _ = print_json(&value);
            outcome.exit
        }
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_grant_cli(profile: &Path, workspace: &Path) -> i32 {
    match seed_house_grants(profile, workspace) {
        Ok(()) => print_json(&serde_json::json!({"seeded": true, "actor": HOUSE_ACTOR})),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_retain_cli(profile: &Path) -> i32 {
    match retain_profile(profile) {
        Ok(report) => print_json(&serde_json::json!({
            "logs_pruned": report.logs_pruned,
            "ledger_rows_pruned": report.ledger_rows_pruned,
            "conversations_pruned": report.conversations_pruned,
        })),
        Err(error) => fail(&error),
    }
}

fn fail(error: &str) -> i32 {
    eprintln!("error: {error}");
    1
}

fn print_json(value: &serde_json::Value) -> i32 {
    match serde_json::to_string(value) {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(error) => fail(&error.to_string()),
    }
}

fn tenant_id(raw: Option<&str>) -> Result<String, String> {
    let id = raw.unwrap_or("local");
    let ok = !id.is_empty()
        && id.len() <= 40
        && !id.starts_with('-')
        && id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if !ok {
        return Err(format!(
            "tenant id '{id}' must be lowercase letters, digits, and hyphens"
        ));
    }
    Ok(id.to_string())
}

fn require_chess(agent: Option<&str>) -> Result<(), String> {
    match agent.unwrap_or(HOUSE_AGENT) {
        HOUSE_AGENT => Ok(()),
        other => Err(format!(
            "the local runner hosts {HOUSE_AGENT} only, not {other}"
        )),
    }
}

fn house_names(tenant: &str) -> HouseNames {
    HouseNames {
        network: format!("plexi-house-{tenant}"),
        edge: format!("plexi-house-edge-{tenant}"),
        relay: format!("plexi-house-relay-{tenant}"),
        agent: format!("plexi-house-agent-{tenant}"),
        volume: format!("plexi-tenant-{tenant}"),
        relay_volume: format!("plexi-relay-{tenant}"),
    }
}

fn scope_grant(workspace: &Path, policy: &ToolPolicy) -> GrantRecord {
    let binding = crate::broker::ExactBinding {
        actor_type: ActorType::Agent,
        actor_id: HOUSE_ACTOR.to_string(),
        actor_scope: ActorScope::Workspace,
        trust_origin: "host".to_string(),
        workspace_root: workspace.to_path_buf(),
        target_type: TargetType::HostTool,
        target_id: policy.tool.to_string(),
        resource_scope: crate::broker::ResourceScope::Workspace,
        resource_id: None,
        args_fingerprint: "scope".to_string(),
        session_id: None,
        package_id: HOUSE_PACKAGE.to_string(),
        instance_id: Some(0),
        context_id: Some(0),
        call_id: String::new(),
        operation_id: String::new(),
    };
    let mut record = GrantRecord::from_binding(
        &binding,
        policy.decision,
        GrantDuration::Always,
        GrantSource::User,
        &format!("grant_{}", uuid::Uuid::new_v4()),
    );
    record.args_unbound = true;
    record
}

pub fn seed_house_grants(profile: &Path, workspace: &Path) -> Result<(), String> {
    fs::create_dir_all(profile)
        .map_err(|error| format!("create {}: {error}", profile.display()))?;
    fs::create_dir_all(workspace)
        .map_err(|error| format!("create {}: {error}", workspace.display()))?;
    let gate = PermissionMonitor::for_profile(profile);
    {
        let mut store = gate.store();
        for policy in HOUSE_POLICIES {
            store.record(scope_grant(workspace, policy));
        }
        store.save();
    }
    log::info!(
        "cloud agent: seeded house grants actor={HOUSE_ACTOR} profile={}",
        profile.display()
    );
    Ok(())
}

pub fn admit_house_tool(
    profile: &Path,
    workspace: &Path,
    tool: &str,
    input: &str,
    actor: Option<&str>,
) -> Result<AdmitOutcome, String> {
    if tool.is_empty() || tool.len() > 128 {
        return Err("tool name is empty or too long".into());
    }
    let actor = actor.unwrap_or(HOUSE_ACTOR);
    let gate = PermissionMonitor::for_profile(profile);
    let call_id = format!("call-{}", uuid::Uuid::new_v4());
    let admission = gate.admit(AdmitRequest {
        call_id: &call_id,
        tool,
        input_json: input,
        actor_type: ActorType::Agent,
        actor_id: actor,
        actor_scope: ActorScope::Workspace,
        trust_origin: "host",
        workspace_root: workspace,
        context_id: 0,
        package_id: HOUSE_PACKAGE,
        instance_id: 0,
        target_type: TargetType::HostTool,
    });
    let outcome = match admission {
        Admission::Proceed { .. } => AdmitOutcome {
            decision: "allow",
            code: "ok",
            exit: 0,
        },
        Admission::Denied { code } => AdmitOutcome {
            decision: "deny",
            code,
            exit: 3,
        },
        Admission::Required { .. } => AdmitOutcome {
            decision: "ask",
            code: "waiting_for_permission",
            exit: 4,
        },
    };
    log::info!(
        "cloud agent: admit actor={actor} tool={tool} decision={}",
        outcome.decision
    );
    Ok(outcome)
}

pub fn retain_profile(profile: &Path) -> Result<RetentionReport, String> {
    fs::create_dir_all(profile)
        .map_err(|error| format!("create {}: {error}", profile.display()))?;
    retention::run(&RetentionInput {
        now: chrono::Utc::now(),
        log_today: chrono::Local::now().date_naive(),
        log_retention_days: 30,
        config_dir: profile,
        assistant_dirs: &[],
        retain_local_history: true,
    })
}

fn installed_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| format!("resolve executable: {error}"))?;
    let canon = exe.canonicalize().unwrap_or(exe);
    if !canon.is_file() {
        return Err(format!("binary missing at {}", canon.display()));
    }
    if canon.display().to_string().contains(',') {
        return Err("binary path cannot be mounted (comma in path)".into());
    }
    Ok(canon)
}

fn house_dir() -> Result<PathBuf, String> {
    if let Some(resources) = crate::distribution::resources()? {
        let packaged = resources.join("house-agent");
        if packaged.join("Dockerfile").is_file() {
            return Ok(packaged);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/house-agent");
    if dev.join("Dockerfile").is_file() {
        return Ok(dev);
    }
    Err("house agent image files are not in this install".into())
}

fn chess_package() -> Result<PathBuf, String> {
    if let Some(resources) = crate::distribution::resources()? {
        let packaged = resources.join("agents").join(HOUSE_AGENT);
        if packaged.join("AGENT.md").is_file() {
            return Ok(packaged);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("agents")
        .join(HOUSE_AGENT);
    if dev.join("AGENT.md").is_file() {
        return Ok(dev);
    }
    Err("chess-opponent package is not in this install".into())
}

fn relay_source(house: &Path) -> Result<PathBuf, String> {
    let packaged = house.join("relay.py");
    if packaged.is_file() {
        return Ok(packaged);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/relay/relay.py");
    if dev.is_file() {
        return Ok(dev);
    }
    Err("relay.py is not in this install".into())
}

fn stage_image() -> Result<StagedImage, String> {
    let house = house_dir()?;
    let agent = chess_package()?;
    let relay = relay_source(&house)?;
    let context = std::env::temp_dir().join(format!("plexi-house-build-{}", uuid::Uuid::new_v4()));
    let built = stage_image_files(&house, &agent, &relay, &context);
    if built.is_err() {
        let _ = fs::remove_dir_all(&context);
    }
    built
}

fn stage_image_files(
    house: &Path,
    agent: &Path,
    relay: &Path,
    context: &Path,
) -> Result<StagedImage, String> {
    fs::create_dir_all(context.join("static"))
        .map_err(|error| format!("create build context: {error}"))?;
    fs::create_dir_all(context.join("agent"))
        .map_err(|error| format!("create build context: {error}"))?;
    copy_file(&house.join("Dockerfile"), &context.join("Dockerfile"))?;
    copy_file(&house.join("runner.py"), &context.join("runner.py"))?;
    copy_file(relay, &context.join("relay.py"))?;
    copy_file(
        &house.join("static/index.html"),
        &context.join("static/index.html"),
    )?;
    copy_file(&agent.join("AGENT.md"), &context.join("agent/AGENT.md"))?;
    copy_file(
        &agent.join("settings.toml"),
        &context.join("agent/settings.toml"),
    )?;
    let tag = image_tag(context)?;
    Ok(StagedImage {
        context: context.to_path_buf(),
        tag,
    })
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|error| format!("copy {} to {}: {error}", from.display(), to.display()))
}

fn image_tag(context: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for rel in [
        "Dockerfile",
        "runner.py",
        "relay.py",
        "static/index.html",
        "agent/AGENT.md",
        "agent/settings.toml",
    ] {
        let bytes = fs::read(context.join(rel)).map_err(|error| format!("hash {rel}: {error}"))?;
        hasher.update(rel.as_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    let hex: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("plexi-house-agent:{hex}"))
}

fn agent_env(_names: &HouseNames, tenant: &str) -> Vec<(String, String)> {
    vec![
        ("HOME".into(), "/tenant/home".into()),
        ("TZ".into(), "UTC".into()),
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("PYTHONUNBUFFERED".into(), "1".into()),
        ("RELAY_URL".into(), "http://relay:8080".into()),
        ("TENANT_ID".into(), tenant.to_string()),
        ("AGENT_ID".into(), HOUSE_AGENT.into()),
        ("PLEXI_BIN".into(), "/opt/plexi/plexi".into()),
    ]
}

fn relay_env() -> Vec<(String, String)> {
    vec![
        ("HOME".into(), "/tmp".into()),
        ("TZ".into(), "UTC".into()),
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("PYTHONUNBUFFERED".into(), "1".into()),
        (
            "RELAY_STATE_PATH".into(),
            "/var/lib/plexi-relay/registry.sqlite".into(),
        ),
        ("RELAY_COOKIE_SECURE".into(), "0".into()),
    ]
}

fn push_env(args: &mut Vec<String>, env: &[(String, String)]) {
    for (key, value) in env {
        args.push("-e".into());
        args.push(format!("{key}={value}"));
    }
}

fn network_create_args(network: &str) -> Vec<String> {
    vec![
        "network".into(),
        "create".into(),
        "--internal".into(),
        network.into(),
    ]
}

fn edge_network_args(network: &str) -> Vec<String> {
    vec![
        "network".into(),
        "create".into(),
        "--opt".into(),
        "com.docker.network.bridge.enable_ip_masquerade=false".into(),
        network.into(),
    ]
}

fn relay_attach_args(names: &HouseNames) -> Vec<String> {
    vec![
        "network".into(),
        "connect".into(),
        "--alias".into(),
        "relay".into(),
        names.network.clone(),
        names.relay.clone(),
    ]
}

fn agent_run_args(names: &HouseNames, tenant: &str, binary: &Path, image: &str) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        names.agent.clone(),
        "--network".into(),
        names.network.clone(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp:rw,noexec,nosuid,size=32m".into(),
        "--user".into(),
        TENANT_UID.into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--security-opt".into(),
        "no-new-privileges:true".into(),
        "--memory".into(),
        "512m".into(),
        "--pids-limit".into(),
        "128".into(),
        "--label".into(),
        "plexi.house=agent".into(),
        "--label".into(),
        format!("plexi.tenant={tenant}"),
        "--mount".into(),
        format!("type=volume,source={},target=/tenant", names.volume),
        "--mount".into(),
        format!(
            "type=bind,source={},target=/opt/plexi/plexi,readonly",
            binary.display()
        ),
    ];
    push_env(&mut args, &agent_env(names, tenant));
    args.push(image.into());
    args.push("python".into());
    args.push("/opt/house/runner.py".into());
    args
}

fn relay_run_args(names: &HouseNames, tenant: &str, image: &str) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        names.relay.clone(),
        "--network".into(),
        names.edge.clone(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp:rw,noexec,nosuid,size=32m".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--security-opt".into(),
        "no-new-privileges:true".into(),
        "--memory".into(),
        "256m".into(),
        "--publish".into(),
        "127.0.0.1::8080".into(),
        "--label".into(),
        "plexi.house=relay".into(),
        "--label".into(),
        format!("plexi.tenant={tenant}"),
        "--mount".into(),
        format!(
            "type=volume,source={},target=/var/lib/plexi-relay",
            names.relay_volume
        ),
    ];
    push_env(&mut args, &relay_env());
    args.push(image.into());
    args.push("python".into());
    args.push("/opt/house/relay.py".into());
    args.push("--host".into());
    args.push("0.0.0.0".into());
    args.push("--port".into());
    args.push("8080".into());
    args.push("--phone-static".into());
    args.push("/opt/house/static".into());
    args.push("--public-origin".into());
    args.push("http://127.0.0.1".into());
    args
}

fn validate_mounts(args: &[String], binary: &Path) -> Result<(), String> {
    let mut index = 0;
    while index < args.len() {
        let item = &args[index];
        if item.contains("docker.sock") {
            return Err("refusing to mount the docker socket".into());
        }
        if item == "--mount" {
            let spec = args
                .get(index + 1)
                .ok_or_else(|| "docker --mount is missing its value".to_string())?;
            if spec.contains("docker.sock") {
                return Err("refusing to mount the docker socket".into());
            }
            check_mount(spec, binary)?;
        }
        index += 1;
    }
    Ok(())
}

fn check_mount(spec: &str, binary: &Path) -> Result<(), String> {
    let mut kind = "";
    let mut source = "";
    let mut readonly = false;
    for part in spec.split(',') {
        if let Some(value) = part.strip_prefix("type=") {
            kind = value;
        } else if let Some(value) = part.strip_prefix("source=") {
            source = value;
        } else if part == "readonly" {
            readonly = true;
        }
    }
    match kind {
        "volume" if source.starts_with("plexi-tenant-") || source.starts_with("plexi-relay-") => {
            Ok(())
        }
        "bind" if Path::new(source) == binary && readonly => Ok(()),
        _ => Err(format!("refusing mount {spec}")),
    }
}

fn run_docker(args: &[String]) -> Result<std::process::Output, String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|error| {
            format!(
                "docker {}: {error}",
                args.first().map(String::as_str).unwrap_or("run")
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let short = if detail.len() > 400 {
            &detail[..400]
        } else {
            detail
        };
        return Err(format!(
            "docker {} failed: {short}",
            args.first().map(String::as_str).unwrap_or("run")
        ));
    }
    Ok(output)
}

fn docker_ok(args: &[&str]) -> bool {
    Command::new("docker")
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn ensure_image(staged: &StagedImage) -> Result<(), String> {
    if docker_ok(&["image", "inspect", &staged.tag]) {
        log::info!("cloud agent: image {} already built", staged.tag);
        return Ok(());
    }
    log::info!("cloud agent: building image {}", staged.tag);
    run_docker(&[
        "build".into(),
        "-t".into(),
        staged.tag.clone(),
        staged.context.display().to_string(),
    ])?;
    Ok(())
}

fn container_state(name: &str) -> Option<bool> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.trim() == "true")
}

fn published_port(container: &str) -> Result<u16, String> {
    let output = run_docker(&["port".into(), container.into(), "8080/tcp".into()])?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_published_port(&text).ok_or_else(|| format!("relay published no port: {text}"))
}

fn parse_published_port(text: &str) -> Option<u16> {
    let mut fallback = None;
    for line in text.lines() {
        let line = line.trim();
        let port = line.rsplit(':').next()?.parse::<u16>().ok()?;
        if line.starts_with("127.0.0.1:") {
            return Some(port);
        }
        fallback = Some(port);
    }
    fallback
}

fn wait_health(port: u16) -> Result<(), String> {
    let address: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|error| format!("relay address: {error}"))?;
    for _ in 0..60 {
        if health_ok(address) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!("relay on {address} did not become healthy"))
}

fn health_ok(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(1)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buffer = [0u8; 512];
    let Ok(count) = stream.read(&mut buffer) else {
        return false;
    };
    let text = String::from_utf8_lossy(&buffer[..count]);
    text.contains("200") && text.contains("\"ok\"")
}

fn wait_pairing(container: &str) -> Result<serde_json::Value, String> {
    for _ in 0..60 {
        let output = Command::new("docker")
            .args(["exec", container, "cat", "/tenant/pairing.json"])
            .output();
        if let Ok(output) = output {
            if output.status.success() {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if value.get("code").and_then(|item| item.as_str()).is_some() {
                        return Ok(value);
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err("house agent did not publish a pairing code".into())
}

fn state_path(tenant: &str) -> PathBuf {
    crate::config::config_dir()
        .join("cloud-house")
        .join(format!("{tenant}.json"))
}

fn write_state(tenant: &str, value: &serde_json::Value) -> Result<(), String> {
    let path = state_path(tenant);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    crate::platform::fs::atomic_write(&path, body.as_bytes())
}

fn remove_state(tenant: &str) {
    let _ = fs::remove_file(state_path(tenant));
}

fn run_house(agent: Option<&str>, tenant: Option<&str>) -> Result<serde_json::Value, String> {
    require_chess(agent)?;
    let tenant = tenant_id(tenant)?;
    let names = house_names(&tenant);
    if container_state(&names.agent) == Some(true) || container_state(&names.relay) == Some(true) {
        return Err(format!(
            "tenant {tenant} is already running; stop it before starting again"
        ));
    }
    remove_runtime(&names);
    let binary = installed_binary()?;
    let staged = stage_image()?;
    ensure_image(&staged)?;
    let agent_args = agent_run_args(&names, &tenant, &binary, &staged.tag);
    let relay_args = relay_run_args(&names, &tenant, &staged.tag);
    validate_mounts(&agent_args, &binary)?;
    validate_mounts(&relay_args, &binary)?;
    log::info!(
        "cloud agent: run tenant={tenant} agent={HOUSE_AGENT} network=internal volume={}",
        names.volume
    );
    if let Err(error) = create_networks(&names) {
        remove_runtime(&names);
        return Err(error);
    }
    if let Err(error) = start_containers(&names, &relay_args, &agent_args, &staged.tag) {
        remove_runtime(&names);
        return Err(error);
    }
    if let Err(error) = inject_tenant_vault(&names.agent, &tenant) {
        remove_runtime(&names);
        return Err(error);
    }
    let port = published_port(&names.relay)?;
    wait_health(port)?;
    let pairing = wait_pairing(&names.agent)?;
    let value = serde_json::json!({
        "tenant": tenant,
        "agent": HOUSE_AGENT,
        "running": true,
        "relay_port": port,
        "network": names.network,
        "volume": names.volume,
        "pairing_id": pairing.get("pairing_id").and_then(|item| item.as_str()).unwrap_or(""),
        "pairing_code": pairing.get("code").and_then(|item| item.as_str()).unwrap_or(""),
    });
    write_state(&tenant, &value)?;
    log::info!("cloud agent: ready tenant={tenant} relay_port={port}");
    Ok(value)
}

fn create_networks(names: &HouseNames) -> Result<(), String> {
    run_docker(&network_create_args(&names.network))?;
    run_docker(&edge_network_args(&names.edge))?;
    Ok(())
}

fn chown_volume_args(volume: &str, image: &str) -> Vec<String> {
    vec![
        "run".into(),
        "--rm".into(),
        "--user".into(),
        "0:0".into(),
        "--network".into(),
        "none".into(),
        "--read-only".into(),
        "--mount".into(),
        format!("type=volume,source={volume},target=/tenant"),
        "--entrypoint".into(),
        "python".into(),
        image.into(),
        "-c".into(),
        "import os\nuid, gid = 65532, 65532\nos.chown('/tenant', uid, gid)\nos.chmod('/tenant', 0o700)\nfor root, dirs, files in os.walk('/tenant'):\n    for name in dirs + files:\n        os.chown(os.path.join(root, name), uid, gid)\n".into(),
    ]
}

fn start_containers(
    names: &HouseNames,
    relay_args: &[String],
    agent_args: &[String],
    image: &str,
) -> Result<(), String> {
    log::info!(
        "cloud agent: chown tenant volume uid={TENANT_UID} volume={}",
        names.volume
    );
    run_docker(&chown_volume_args(&names.volume, image))?;
    run_docker(relay_args)?;
    run_docker(&relay_attach_args(names))?;
    run_docker(agent_args)?;
    Ok(())
}

fn remove_runtime(names: &HouseNames) {
    let _ = Command::new("docker")
        .args(["rm", "-f", &names.agent, &names.relay])
        .output();
    let _ = Command::new("docker")
        .args(["network", "rm", &names.network, &names.edge])
        .output();
}

fn stop_house(tenant: Option<&str>) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    let names = house_names(&tenant);
    remove_runtime(&names);
    remove_state(&tenant);
    log::info!("cloud agent: stop tenant={tenant}");
    Ok(serde_json::json!({
        "tenant": tenant,
        "agent": HOUSE_AGENT,
        "running": false,
    }))
}

fn status_house(tenant: Option<&str>) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    let names = house_names(&tenant);
    let running = container_state(&names.agent) == Some(true);
    log::info!("cloud agent: status tenant={tenant} running={running}");
    let mut value = serde_json::json!({
        "tenant": tenant,
        "agent": HOUSE_AGENT,
        "running": running,
        "network": names.network,
        "volume": names.volume,
    });
    if let Ok(raw) = fs::read_to_string(state_path(&tenant)) {
        if let Ok(saved) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(port) = saved.get("relay_port") {
                value["relay_port"] = port.clone();
            }
        }
    }
    Ok(value)
}

const WRITE_DECISION: &str = "\
import json, os, sys\n\
from pathlib import Path\n\
body = json.load(sys.stdin)\n\
path = Path('/tenant/pairing-decision.json')\n\
fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)\n\
with os.fdopen(fd, 'w', encoding='utf-8') as handle:\n\
    json.dump(body, handle)\n\
";

const WRITE_VAULT: &str = "\
import os, sys\n\
from pathlib import Path\n\
path = Path('/tenant/vault')\n\
path.mkdir(parents=True, exist_ok=True)\n\
target = path / 'model'\n\
fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)\n\
with os.fdopen(fd, 'wb') as handle:\n\
    handle.write(sys.stdin.buffer.read())\n\
";

fn exec_stdin(container: &str, script: &str, bytes: &[u8]) -> Result<(), String> {
    let mut child = Command::new("docker")
        .args([
            "exec", "-i", "--user", TENANT_UID, container, "python", "-c", script,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("docker exec: {error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "docker exec stdin".to_string())?
        .write_all(bytes)
        .map_err(|error| format!("write docker exec stdin: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("docker exec: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("docker exec failed: {}", detail.trim()));
    }
    Ok(())
}

fn inject_tenant_vault(container: &str, tenant: &str) -> Result<(), String> {
    let vault = crate::cloud::vault::TenantVault::for_profile();
    let Some(secret) = vault.get(tenant)? else {
        log::info!("cloud agent: vault absent tenant={tenant}");
        return Ok(());
    };
    exec_stdin(container, WRITE_VAULT, secret.as_bytes())?;
    let fingerprint = crate::cloud::vault::fingerprint(secret.as_str());
    log::info!("cloud agent: vault injected tenant={tenant} fingerprint={fingerprint}");
    Ok(())
}

fn clear_injected_vault(container: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .args([
            "exec",
            "--user",
            TENANT_UID,
            container,
            "python",
            "-c",
            "from pathlib import Path\nPath('/tenant/vault/model').unlink(missing_ok=True)\n",
        ])
        .output()
        .map_err(|error| format!("docker exec: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("clear vault failed: {}", detail.trim()));
    }
    Ok(())
}

pub fn cloud_agent_pending_cli(tenant: Option<&str>) -> i32 {
    match pending_house(tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_approve_cli(tenant: Option<&str>, pairing_id: &str) -> i32 {
    match decide_house(tenant, pairing_id, "approve") {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_deny_cli(tenant: Option<&str>, pairing_id: &str) -> i32 {
    match decide_house(tenant, pairing_id, "deny") {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_vault_set_cli(tenant: Option<&str>) -> i32 {
    match vault_write(tenant, false) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_vault_rotate_cli(tenant: Option<&str>) -> i32 {
    match vault_write(tenant, true) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_vault_revoke_cli(tenant: Option<&str>) -> i32 {
    match vault_revoke(tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

pub fn cloud_agent_vault_status_cli(tenant: Option<&str>) -> i32 {
    match vault_status(tenant) {
        Ok(value) => print_json(&value),
        Err(error) => fail(&error),
    }
}

fn pending_house(tenant: Option<&str>) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    let names = house_names(&tenant);
    if container_state(&names.agent) != Some(true) {
        log::info!("cloud agent: needs you tenant={tenant} pending=0");
        return Ok(serde_json::json!({"tenant": tenant, "needs_you": []}));
    }
    let output = Command::new("docker")
        .args([
            "exec",
            "--user",
            TENANT_UID,
            &names.agent,
            "python",
            "-c",
            "from pathlib import Path\npath = Path('/tenant/needs-you.json')\nprint(path.read_text(encoding='utf-8') if path.is_file() else '')\n",
        ])
        .output()
        .map_err(|error| format!("docker exec: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("needs you: {}", detail.trim()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let body = if text.trim().is_empty() {
        serde_json::json!({"needs_you": []})
    } else {
        serde_json::from_str(text.trim()).unwrap_or_else(|_| serde_json::json!({"needs_you": []}))
    };
    let count = body
        .get("needs_you")
        .and_then(|item| item.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    log::info!("cloud agent: needs you tenant={tenant} pending={count}");
    let mut value = body;
    if let Some(object) = value.as_object_mut() {
        object.insert("tenant".into(), serde_json::Value::String(tenant));
    }
    Ok(value)
}

fn decide_house(
    tenant: Option<&str>,
    pairing_id: &str,
    decision: &str,
) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    if pairing_id.is_empty() {
        return Err("pairing id is required".into());
    }
    let names = house_names(&tenant);
    if container_state(&names.agent) != Some(true) {
        return Err(format!("tenant {tenant} is not running"));
    }
    let body = serde_json::json!({"pairing_id": pairing_id, "decision": decision});
    let bytes = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    exec_stdin(&names.agent, WRITE_DECISION, &bytes)?;
    log::info!("cloud agent: {decision} tenant={tenant} pairing_id={pairing_id}");
    Ok(serde_json::json!({
        "tenant": tenant,
        "pairing_id": pairing_id,
        "decision": decision,
    }))
}

fn vault_write(tenant: Option<&str>, rotate: bool) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    let secret = crate::cloud::vault::read_secret_from_stdin()?;
    let vault = crate::cloud::vault::TenantVault::for_profile();
    let fingerprint = if rotate {
        vault.rotate(&tenant, &secret)?
    } else {
        vault.set(&tenant, &secret)?
    };
    if container_state(&house_names(&tenant).agent) == Some(true) {
        exec_stdin(&house_names(&tenant).agent, WRITE_VAULT, secret.as_bytes())?;
        log::info!("cloud agent: vault injected tenant={tenant} fingerprint={fingerprint}");
    }
    Ok(serde_json::json!({
        "tenant": tenant,
        "present": true,
        "fingerprint": fingerprint,
    }))
}

fn vault_revoke(tenant: Option<&str>) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    let vault = crate::cloud::vault::TenantVault::for_profile();
    vault.revoke(&tenant)?;
    let names = house_names(&tenant);
    if container_state(&names.agent) == Some(true) {
        clear_injected_vault(&names.agent)?;
    }
    Ok(serde_json::json!({
        "tenant": tenant,
        "present": false,
        "fingerprint": null,
    }))
}

fn vault_status(tenant: Option<&str>) -> Result<serde_json::Value, String> {
    let tenant = tenant_id(tenant)?;
    crate::cloud::vault::TenantVault::for_profile().status(&tenant)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("plexi-house-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn house_gate_allows_a_turn_and_denies_a_play() {
        let root = temp_dir("gate");
        let profile = root.join("profile");
        let workspace = root.join("workspace");
        seed_house_grants(&profile, &workspace).unwrap();
        let allowed = admit_house_tool(
            &profile,
            &workspace,
            "agent.turn",
            r#"{"text":"hello"}"#,
            None,
        )
        .unwrap();
        assert_eq!(allowed.decision, "allow");
        assert_eq!(allowed.exit, 0);
        let denied = admit_house_tool(&profile, &workspace, "app.chess.play", "{}", None).unwrap();
        assert_eq!(denied.decision, "deny");
        assert_eq!(denied.code, "permission_denied");
        assert_eq!(denied.exit, 3);
        let secret =
            admit_house_tool(&profile, &workspace, "host.secret.read", "{}", None).unwrap();
        assert_eq!(secret.decision, "deny");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn house_plan_mounts_only_the_tenant_volume_and_the_binary() {
        let names = house_names("local");
        let binary = PathBuf::from("/opt/plexi/plexi");
        let agent = agent_run_args(&names, "local", &binary, "plexi-house-agent:test");
        let relay = relay_run_args(&names, "local", "plexi-house-agent:test");
        validate_mounts(&agent, &binary).unwrap();
        validate_mounts(&relay, &binary).unwrap();
        let network = network_create_args(&names.network);
        assert!(network.iter().any(|arg| arg == "--internal"));
        let edge = edge_network_args(&names.edge);
        assert!(!edge.iter().any(|arg| arg == "--internal"));
        assert!(edge
            .iter()
            .any(|arg| arg.contains("enable_ip_masquerade=false")));
        let attach = relay_attach_args(&names);
        assert!(attach.iter().any(|arg| arg == "relay"));
        assert!(agent
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == names.network));
        assert!(relay
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == names.edge));
        assert!(!agent.iter().any(|arg| arg == "--publish"));
        assert!(agent
            .windows(2)
            .any(|pair| pair[0] == "--user" && pair[1] == "65532:65532"));
        let chown = chown_volume_args(&names.volume, "plexi-house-agent:test");
        assert!(chown
            .windows(2)
            .any(|pair| pair[0] == "--user" && pair[1] == "0:0"));
        assert!(chown
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "none"));
        assert!(!chown
            .iter()
            .any(|arg| arg.contains("docker.sock") || arg.contains("/opt/plexi/plexi")));
        assert!(!WRITE_VAULT.contains("sk-"));
        assert!(!WRITE_DECISION.contains("pairing_code"));
        let env = agent_env(&names, "local");
        assert!(env
            .iter()
            .any(|(key, value)| key == "RELAY_URL" && value == "http://relay:8080"));
        assert!(agent.iter().any(|arg| arg == "--read-only"));
        assert!(agent
            .iter()
            .any(|arg| arg.contains("type=volume,source=plexi-tenant-local")));
        assert!(relay.iter().any(|arg| arg == "127.0.0.1::8080"));
        assert!(!agent.iter().any(|arg| arg.contains("docker.sock")));
        assert!(agent.iter().any(|arg| arg == "HOME=/tenant/home"));
        assert!(!agent
            .iter()
            .any(|arg| arg.contains("/home/ubuntu") || arg.contains("/Users/")));
        let keys: Vec<&str> = env.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "HOME",
                "TZ",
                "PYTHONDONTWRITEBYTECODE",
                "PYTHONUNBUFFERED",
                "RELAY_URL",
                "TENANT_ID",
                "AGENT_ID",
                "PLEXI_BIN",
            ]
        );
        assert!(validate_mounts(
            &[
                "--mount".into(),
                "type=bind,source=/home/user/.ssh,target=/ssh".into()
            ],
            &binary
        )
        .is_err());
    }

    #[test]
    fn house_retention_drops_rows_older_than_thirty_days() {
        let root = temp_dir("retain");
        let today = chrono::Local::now().date_naive();
        let old_day = (today - chrono::Duration::days(31))
            .format("%Y-%m-%d")
            .to_string();
        let kept_day = (today - chrono::Duration::days(29))
            .format("%Y-%m-%d")
            .to_string();
        fs::write(root.join(format!("plexi-{old_day}.log")), "old").unwrap();
        fs::write(root.join(format!("plexi-{kept_day}.log")), "kept").unwrap();
        let old_ts = (chrono::Utc::now() - chrono::Duration::days(31))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let kept_ts = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        fs::write(
            root.join("ai-ledger.jsonl"),
            format!(
                "{{\"ts\":\"{old_ts}\",\"marker\":\"old\"}}\n{{\"ts\":\"{kept_ts}\",\"marker\":\"kept\"}}\nnot-json\n"
            ),
        )
        .unwrap();
        let report = retain_profile(&root).unwrap();
        assert!(report.logs_pruned >= 1);
        assert!(report.ledger_rows_pruned >= 1);
        assert!(!root.join(format!("plexi-{old_day}.log")).exists());
        assert!(root.join(format!("plexi-{kept_day}.log")).exists());
        let ledger = fs::read_to_string(root.join("ai-ledger.jsonl")).unwrap();
        assert!(!ledger.contains("\"old\""));
        assert!(ledger.contains("\"kept\""));
        assert!(ledger.contains("not-json"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn house_published_port_prefers_loopback() {
        assert_eq!(
            parse_published_port("0.0.0.0:1\n127.0.0.1:4321\n"),
            Some(4321)
        );
    }
}
