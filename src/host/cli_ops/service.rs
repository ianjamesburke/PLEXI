//! Per-pane service for `cli_operation` effects: validation, admission through
//! the permission monitor, parking while a human decides, and bounded launch.
//!
//! One request, one reply. Every reply is a `cli_operation_result` message that
//! echoes the request id, either `ok` with bounded data or a typed failure. No
//! failure path launches a program, and every launch path passes through
//! [`CliOpService::admit_and_launch`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::app::cli_operation::{hex, ArgError, CliOperationDecl, OperationMode, OutputFormat};
use crate::broker::gate::{Admission, AdmitRequest, ApprovalChoice, PermissionMonitor};
use crate::broker::{ActorScope, ActorType, TargetType};

use super::exec::{
    self, Completion, ExecError, LaunchSpec, Limits, Outcome, RunningOperation, Wake,
};

/// Requests one pane may have parked or running at once.
pub const MAX_OUTSTANDING_PER_PANE: usize = 2;
/// Programs running at once across every pane.
pub const MAX_RUNNING_GLOBAL: usize = 8;
/// How long an approval request waits for a human before the call fails.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const STDERR_IN_ERROR: usize = 512;
const MAX_REQUEST_ID_LEN: usize = 128;

static RUNNING_GLOBAL: AtomicUsize = AtomicUsize::new(0);

/// Typed failure codes a guest can match on.
pub mod code {
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const DISABLED: &str = "disabled";
    pub const CAPABILITY_MISSING: &str = "capability_missing";
    pub const BUSY: &str = "busy";
    pub const UNKNOWN_OPERATION: &str = "unknown_operation";
    pub const PACKAGE_INVALID: &str = "package_invalid";
    pub const INVALID_ARGUMENT: &str = "invalid_argument";
    pub const FLAG_LIKE_ARGUMENT: &str = "flag_like_argument";
    pub const EXECUTABLE_MISSING: &str = "executable_missing";
    pub const EXECUTABLE_MISMATCH: &str = "executable_mismatch";
    pub const DENIED: &str = "denied";
    pub const APPROVAL_TIMEOUT: &str = "approval_timeout";
    pub const AUDIT_FAILED: &str = "audit_failed";
    pub const LAUNCH_FAILED: &str = "launch_failed";
    pub const TIMEOUT: &str = "timeout";
    pub const OUTPUT_TOO_LARGE: &str = "output_too_large";
    pub const CANCELLED: &str = "cancelled";
    pub const NONZERO_EXIT: &str = "nonzero_exit";
    pub const BAD_OUTPUT: &str = "bad_output";
    pub const EXECUTABLE_NOT_SELF_CONTAINED: &str = "executable_not_self_contained";
}

/// What the host, never the guest, knows about the asking pane.
#[derive(Debug, Clone)]
pub struct PaneIdentity {
    pub plugin_id: String,
    pub plugin_dir: PathBuf,
    pub workspace_root: PathBuf,
    pub pane_id: u64,
    pub context_id: u64,
    /// Manifest membership of `cli.operations`. Necessary, never sufficient.
    pub has_capability: bool,
}

/// Host-owned configuration of one service call, injectable for tests.
pub struct Env<'a> {
    pub monitor: &'a PermissionMonitor,
    pub search_dirs: &'a [PathBuf],
    pub seal_base: &'a Path,
    pub wake: &'a Wake,
    pub enabled: bool,
    pub experiment_message: &'a str,
    pub approval_timeout: Duration,
}

struct Admitted {
    decl: CliOperationDecl,
    values: Map<String, Value>,
    argv_check: Vec<std::ffi::OsString>,
    identity: exec::ExecutableIdentity,
    binding: String,
    package_digest: String,
    workspace: PathBuf,
}

struct Running {
    op: RunningOperation,
    actor: String,
    decl: CliOperationDecl,
    call_id: String,
    grant_id: String,
    fingerprint: String,
    resource: String,
    _slot: RunningSlot,
}

/// Holds one unit of the global running budget.
struct RunningSlot;

impl RunningSlot {
    fn acquire() -> Option<Self> {
        let mut current = RUNNING_GLOBAL.load(Ordering::SeqCst);
        loop {
            if current >= MAX_RUNNING_GLOBAL {
                return None;
            }
            match RUNNING_GLOBAL.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(Self),
                Err(seen) => current = seen,
            }
        }
    }
}

impl Drop for RunningSlot {
    fn drop(&mut self) {
        RUNNING_GLOBAL.fetch_sub(1, Ordering::SeqCst);
    }
}

enum Stage {
    Parked {
        pending_id: String,
        deadline: Instant,
    },
    Running(Box<Running>),
}

struct Entry {
    request_id: String,
    operation_id: String,
    values: Map<String, Value>,
    stage: Stage,
}

#[derive(Default)]
pub struct CliOpService {
    entries: Vec<Entry>,
}

fn failure(
    request_id: &str,
    operation_id: &str,
    error_code: &str,
    message: impl Into<String>,
) -> Value {
    let message = message.into();
    log::info!(
        "cli_ops: request={request_id} operation={operation_id} failed code={error_code}: {message}"
    );
    json!({
        "type": "cli_operation_result",
        "request_id": request_id,
        "operation_id": operation_id,
        "ok": false,
        "error_code": error_code,
        "error": message,
    })
}

fn success(request_id: &str, operation_id: &str, data: Value, bytes: usize) -> Value {
    log::info!("cli_ops: request={request_id} operation={operation_id} ok bytes={bytes}");
    json!({
        "type": "cli_operation_result",
        "request_id": request_id,
        "operation_id": operation_id,
        "ok": true,
        "data": data,
    })
}

fn exec_failure(error: &ExecError) -> (&'static str, String) {
    match error {
        ExecError::NotFound { .. } => (code::EXECUTABLE_MISSING, error.to_string()),
        ExecError::Refused { .. } | ExecError::ContentMismatch { .. } => {
            (code::EXECUTABLE_MISMATCH, error.to_string())
        }
        ExecError::Io { .. } => (code::LAUNCH_FAILED, error.to_string()),
    }
}

fn arg_failure(error: &ArgError) -> (&'static str, String) {
    match error {
        ArgError::FlagLike(_) => (code::FLAG_LIKE_ARGUMENT, error.to_string()),
        ArgError::Definition(_) => (code::PACKAGE_INVALID, error.to_string()),
        _ => (code::INVALID_ARGUMENT, error.to_string()),
    }
}

impl CliOpService {
    pub fn has_work(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Handle one `cli_operation` message. Returns replies to deliver now; a
    /// parked or running request replies later from [`Self::poll`].
    pub fn submit(&mut self, who: &PaneIdentity, env: &Env<'_>, message: &Value) -> Vec<Value> {
        let request_id = message
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let operation_id = message
            .get("operation_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_LEN {
            return vec![failure(
                request_id,
                operation_id,
                code::INVALID_REQUEST,
                "request_id must be 1-128 characters",
            )];
        }
        // The guest names an operation and typed values. Nothing else: an
        // executable, argv, environment, directory or shell text is refused
        // outright rather than ignored, so a bug cannot become authority.
        if let Some(obj) = message.as_object() {
            if let Some(extra) = obj.keys().find(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "request_id" | "operation_id" | "values"
                )
            }) {
                return vec![failure(
                    request_id,
                    operation_id,
                    code::INVALID_REQUEST,
                    format!("'{extra}' is not accepted: a request carries only request_id, operation_id and values"),
                )];
            }
        }
        let Some(values) = message.get("values").and_then(Value::as_object) else {
            return vec![failure(
                request_id,
                operation_id,
                code::INVALID_REQUEST,
                "values must be an object",
            )];
        };
        if self
            .entries
            .iter()
            .any(|entry| entry.request_id == request_id)
        {
            return vec![failure(
                request_id,
                operation_id,
                code::INVALID_REQUEST,
                "request_id is already in flight",
            )];
        }
        if !env.enabled {
            return vec![failure(
                request_id,
                operation_id,
                code::DISABLED,
                env.experiment_message,
            )];
        }
        if !who.has_capability {
            return vec![failure(
                request_id,
                operation_id,
                code::CAPABILITY_MISSING,
                "missing capability cli.operations",
            )];
        }
        if self.entries.len() >= MAX_OUTSTANDING_PER_PANE {
            return vec![failure(
                request_id,
                operation_id,
                code::BUSY,
                format!("at most {MAX_OUTSTANDING_PER_PANE} operations may be pending or running per plugin"),
            )];
        }
        log::info!(
            "cli_ops: submit plugin={} request={request_id} operation={operation_id}",
            who.plugin_id
        );
        let values = values.clone();
        match self.admit_and_launch(who, env, request_id, operation_id, &values) {
            Ok(Step::Park(pending_id)) => {
                self.entries.push(Entry {
                    request_id: request_id.to_string(),
                    operation_id: operation_id.to_string(),
                    values,
                    stage: Stage::Parked {
                        pending_id,
                        deadline: Instant::now() + env.approval_timeout,
                    },
                });
                Vec::new()
            }
            Ok(Step::Run(running)) => {
                self.entries.push(Entry {
                    request_id: request_id.to_string(),
                    operation_id: operation_id.to_string(),
                    values,
                    stage: Stage::Running(running),
                });
                Vec::new()
            }
            Err(reply) => vec![reply],
        }
    }

    /// Cancel one request. A parked request fails now; a running one is killed
    /// and replies `cancelled` from [`Self::poll`].
    pub fn cancel(&mut self, request_id: &str) -> Vec<Value> {
        let Some(index) = self.entries.iter().position(|e| e.request_id == request_id) else {
            log::info!("cli_ops: cancel for unknown request={request_id} ignored");
            return Vec::new();
        };
        match &self.entries[index].stage {
            Stage::Running(running) => {
                running.op.cancel();
                Vec::new()
            }
            Stage::Parked { .. } => {
                let entry = self.entries.remove(index);
                vec![failure(
                    &entry.request_id,
                    &entry.operation_id,
                    code::CANCELLED,
                    "cancelled before approval",
                )]
            }
        }
    }

    /// Advance parked and running requests. Safe to call every pass.
    pub fn poll(&mut self, who: &PaneIdentity, env: &Env<'_>) -> Vec<Value> {
        let mut replies = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            let finished = match &self.entries[index].stage {
                Stage::Running(running) => running.op.try_outcome().map(Resolved::Outcome),
                Stage::Parked {
                    pending_id,
                    deadline,
                } => match env.monitor.approval_resolution(pending_id) {
                    Some(ApprovalChoice::Deny | ApprovalChoice::DenyAlways) => {
                        Some(Resolved::Denied)
                    }
                    Some(_) => Some(Resolved::Decided),
                    None if Instant::now() >= *deadline => Some(Resolved::TimedOut),
                    None => None,
                },
            };
            let Some(resolved) = finished else {
                index += 1;
                continue;
            };
            let entry = self.entries.remove(index);
            match resolved {
                Resolved::Outcome(outcome) => {
                    if let Stage::Running(running) = entry.stage {
                        replies.push(finish(
                            env,
                            &entry.request_id,
                            &entry.operation_id,
                            *running,
                            outcome,
                        ));
                    }
                }
                Resolved::Denied => replies.push(failure(
                    &entry.request_id,
                    &entry.operation_id,
                    code::DENIED,
                    "the approval was denied",
                )),
                Resolved::TimedOut => replies.push(failure(
                    &entry.request_id,
                    &entry.operation_id,
                    code::APPROVAL_TIMEOUT,
                    "no decision was made in time",
                )),
                Resolved::Decided => {
                    // The decision changes grants, not facts: re-derive every
                    // binding from the package and executable as they are now,
                    // so an approval can never cover a replaced program.
                    let next = match self.admit_and_launch(
                        who,
                        env,
                        &entry.request_id,
                        &entry.operation_id,
                        &entry.values,
                    ) {
                        Err(reply) => {
                            replies.push(reply);
                            None
                        }
                        Ok(Step::Park(pending_id)) => Some(Stage::Parked {
                            pending_id,
                            deadline: Instant::now() + env.approval_timeout,
                        }),
                        Ok(Step::Run(running)) => Some(Stage::Running(running)),
                    };
                    if let Some(stage) = next {
                        self.entries.insert(index, Entry { stage, ..entry });
                        index += 1;
                    }
                }
            }
        }
        replies
    }

    /// The only path to a launch: validate, bind, admit, audit, consume, seal,
    /// verify, spawn. Each early return launches nothing.
    fn admit_and_launch(
        &self,
        who: &PaneIdentity,
        env: &Env<'_>,
        request_id: &str,
        operation_id: &str,
        values: &Map<String, Value>,
    ) -> Result<Step, Value> {
        let fail = |error_code: &str, message: String| {
            failure(request_id, operation_id, error_code, message)
        };
        let admitted = self
            .prepare(who, env, operation_id, values)
            .map_err(|(c, m)| fail(c, m))?;
        let mode = admitted.decl.authority.mode;
        let tool = match mode {
            OperationMode::Read => "cli.op.read",
            OperationMode::Mutate => "cli.op.mutate",
        };
        let resource = format!("{operation_id}#{}", admitted.binding);
        let input = json!({
            "cli_operation_id": operation_id,
            "binding": resource,
            "values": admitted.values,
        })
        .to_string();
        let call_id = format!("cliop_{request_id}_{}", uuid::Uuid::new_v4().simple());
        let disclosure = Disclosure {
            plugin_id: &who.plugin_id,
            operation_id: &admitted.decl.id,
            mode,
            workspace: &admitted.workspace,
            executable: &admitted.identity.path,
            executable_sha256: &admitted.identity.sha256,
            authority: &admitted.decl.authority,
            changed_since_approval: env.monitor.list_entries().iter().any(|entry| {
                entry.kind == "allow"
                    && entry.actor_id == who.plugin_id
                    && entry.tool == tool
                    && entry.resource_id.as_deref().is_some_and(|id| {
                        id != resource && id.starts_with(&format!("{operation_id}#"))
                    })
            }),
        }
        .text();
        let admission = env.monitor.admit_cli_operation(
            AdmitRequest {
                call_id: &call_id,
                tool,
                input_json: &input,
                actor_type: ActorType::Plugin,
                actor_id: &who.plugin_id,
                actor_scope: ActorScope::User,
                trust_origin: "host",
                workspace_root: &admitted.workspace,
                context_id: who.context_id,
                package_id: &admitted.package_digest,
                instance_id: who.pane_id,
                target_type: TargetType::CliOperation,
            },
            &disclosure,
        );
        match admission {
            Admission::Denied { code: why } => Err(fail(
                code::DENIED,
                format!("denied by permission policy ({why})"),
            )),
            Admission::Required { pending_request_id } => {
                log::info!(
                    "cli_ops: request={request_id} operation={operation_id} awaiting approval pending={pending_request_id}"
                );
                Ok(Step::Park(pending_request_id))
            }
            Admission::Proceed {
                grant_id,
                fingerprint,
                ..
            } => {
                // Audit the use before anything runs; a failed audit write or a
                // revoked grant stops here.
                env.monitor
                    .note_use(
                        &who.plugin_id,
                        &call_id,
                        &grant_id,
                        &fingerprint,
                        &resource,
                        "",
                    )
                    .map_err(|e| {
                        fail(
                            code::AUDIT_FAILED,
                            format!("approval could not be recorded: {e}"),
                        )
                    })?;
                // A one-shot grant is spent by admission, not by completion, so
                // no second request can ride it while the program runs.
                env.monitor.consume_once(&grant_id, None);
                let running = self.launch(who, env, &admitted).map_err(|(c, m)| {
                    let _ = env.monitor.note_outcome(
                        &who.plugin_id,
                        &call_id,
                        &grant_id,
                        &fingerprint,
                        &resource,
                        "",
                        "not_launched",
                        "",
                        "",
                    );
                    fail(c, m)
                })?;
                Ok(Step::Run(Box::new(Running {
                    op: running.0,
                    actor: who.plugin_id.clone(),
                    decl: admitted.decl,
                    call_id,
                    grant_id,
                    fingerprint,
                    resource,
                    _slot: running.1,
                })))
            }
        }
    }

    /// Everything checked before admission: definition, values, package
    /// identity, executable identity and the resulting binding digest.
    fn prepare(
        &self,
        who: &PaneIdentity,
        env: &Env<'_>,
        operation_id: &str,
        values: &Map<String, Value>,
    ) -> Result<Admitted, (&'static str, String)> {
        if !super::plugin_id_is_path_safe(&who.plugin_id) {
            return Err((
                code::PACKAGE_INVALID,
                format!("plugin id '{}' cannot name a private directory", who.plugin_id),
            ));
        }
        let manifest_path = who.plugin_dir.join("manifest.toml");
        let raw = std::fs::read_to_string(&manifest_path).map_err(|e| {
            (
                code::PACKAGE_INVALID,
                format!("cannot read {}: {e}", manifest_path.display()),
            )
        })?;
        let manifest: crate::app::registry::PluginManifest = toml::from_str(&raw).map_err(|e| {
            (
                code::PACKAGE_INVALID,
                format!("manifest.toml does not parse: {e}"),
            )
        })?;
        crate::app::cli_operation::validate_all(&manifest.cli_operations)
            .map_err(|e| (code::PACKAGE_INVALID, e.to_string()))?;
        if manifest.plugin.id != who.plugin_id {
            return Err((
                code::PACKAGE_INVALID,
                format!(
                    "package declares plugin id '{}' but the pane runs '{}'",
                    manifest.plugin.id, who.plugin_id
                ),
            ));
        }
        let decl = manifest
            .cli_operations
            .iter()
            .find(|d| d.id == operation_id)
            .cloned()
            .ok_or_else(|| {
                (
                    code::UNKNOWN_OPERATION,
                    format!("the package declares no operation '{operation_id}'"),
                )
            })?;
        if is_registry_install(&who.plugin_dir)
            && !who
                .plugin_dir
                .join(crate::app::package::INSTALL_COMPLETE_MARKER)
                .is_file()
        {
            return Err((
                code::PACKAGE_INVALID,
                "the installed package is incomplete or its update was interrupted; reinstall it"
                    .to_string(),
            ));
        }
        let argv_check = decl.build_argv(values).map_err(|e| arg_failure(&e))?;
        let package_digest = crate::app::package::content_digest(&who.plugin_dir)
            .map_err(|e| (code::PACKAGE_INVALID, e.to_string()))?;
        let workspace = crate::platform::path::canonical_or_self(&who.workspace_root);
        let path =
            exec::resolve(&decl.executable, env.search_dirs).map_err(|e| exec_failure(&e))?;
        let opened = exec::open_and_identify(&path).map_err(|e| exec_failure(&e))?;
        let identity = opened.identity.clone();
        drop(opened);
        let binding = binding_digest(&package_digest, &workspace, &decl, &identity);
        Ok(Admitted {
            decl,
            values: values.clone(),
            argv_check,
            identity,
            binding,
            package_digest,
            workspace,
        })
    }

    /// Seal the approved bytes and start supervision. Values are validated a
    /// second time here, immediately before launch.
    fn launch(
        &self,
        who: &PaneIdentity,
        env: &Env<'_>,
        admitted: &Admitted,
    ) -> Result<(RunningOperation, RunningSlot), (&'static str, String)> {
        let argv = admitted
            .decl
            .build_argv(&admitted.values)
            .map_err(|e| arg_failure(&e))?;
        if argv != admitted.argv_check {
            return Err((
                code::INVALID_ARGUMENT,
                "argument vector changed between admission and launch".into(),
            ));
        }
        let slot = RunningSlot::acquire().ok_or((
            code::BUSY,
            format!("{MAX_RUNNING_GLOBAL} operations are already running"),
        ))?;
        let opened =
            exec::open_and_identify(&admitted.identity.path).map_err(|e| exec_failure(&e))?;
        let sealed = opened
            .seal(&env.seal_base.join(&who.plugin_id), &admitted.identity.sha256)
            .map_err(|e| exec_failure(&e))?;
        let spec = LaunchSpec {
            sealed,
            display_name: admitted
                .decl
                .executable
                .rsplit('/')
                .next()
                .unwrap_or("program")
                .to_string(),
            argv,
            cwd: admitted.workspace.clone(),
            env: exec::fixed_env(),
            limits: Limits {
                timeout: Duration::from_millis(admitted.decl.timeout_ms),
                max_output: admitted.decl.output.max_bytes,
            },
        };
        let op =
            exec::spawn_supervised(spec, Arc::clone(env.wake)).map_err(|e| exec_failure(&e))?;
        log::info!(
            "cli_ops: plugin={} operation={} launched exe={} sha256={}",
            who.plugin_id,
            admitted.decl.id,
            admitted.identity.path.display(),
            &admitted.identity.sha256[..12]
        );
        Ok((op, slot))
    }
}

enum Step {
    Park(String),
    Run(Box<Running>),
}

enum Resolved {
    Outcome(Outcome),
    Denied,
    TimedOut,
    Decided,
}

fn is_registry_install(plugin_dir: &Path) -> bool {
    let plugins = crate::platform::path::canonical_or_self(&crate::app::registry::plugins_dir());
    crate::platform::path::canonical_or_self(plugin_dir).starts_with(plugins)
}

/// SHA-256 over the canonical JSON of everything an approval binds.
fn binding_digest(
    package_digest: &str,
    workspace: &Path,
    decl: &CliOperationDecl,
    identity: &exec::ExecutableIdentity,
) -> String {
    let canonical = json!({
        "package": package_digest,
        "workspace": workspace.display().to_string(),
        "definition": decl.definition_hash(),
        "mode": decl.authority.mode,
        "executable_path": identity.path.display().to_string(),
        "executable_sha256": identity.sha256,
    });
    hex(&Sha256::digest(canonical.to_string().as_bytes()))
}

/// What the approval surface shows. Built once, from host-verified facts, so
/// the seeded scene and the live prompt cannot drift apart.
pub struct Disclosure<'a> {
    pub plugin_id: &'a str,
    pub operation_id: &'a str,
    pub mode: OperationMode,
    pub workspace: &'a Path,
    pub executable: &'a Path,
    pub executable_sha256: &'a str,
    pub authority: &'a crate::app::cli_operation::AuthorityDecl,
    /// This operation was approved before and something it was approved against
    /// (program, plugin package or definition) is different now.
    pub changed_since_approval: bool,
}

impl Disclosure<'_> {
    /// The plugin id is a claim by the package, not proof of what the program
    /// does, and the native program is not confined by anything declared here.
    pub fn text(&self) -> String {
        let scopes = |items: &[String]| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items.join(", ")
            }
        };
        let changed = if self.changed_since_approval {
            "CHANGED SINCE YOU APPROVED THIS: the program, the plugin package or the operation definition is different. "
        } else {
            ""
        };
        format!(
            "{changed}App \"{plugin}\" (name is self-declared) asks to run operation \"{op}\" ({mode}) in workspace {ws}. \
             Program: {exe} (sha256 {sha}). It declares: read {read}; write {write}; network {net}. \
             This runs a native program with your user account's full authority. Plexi does not confine it, \
             and the declaration above is not enforced.",
            changed = changed,
            plugin = self.plugin_id,
            op = self.operation_id,
            mode = match self.mode {
                OperationMode::Read => "read",
                OperationMode::Mutate => "changes data",
            },
            ws = self.workspace.display(),
            exe = self.executable.display(),
            sha = &self.executable_sha256[..self.executable_sha256.len().min(12)],
            read = scopes(&self.authority.fs_read),
            write = scopes(&self.authority.fs_write),
            net = scopes(&self.authority.net_hosts),
        )
    }
}

/// Turn a finished process into the guest's reply and the audit outcome.
fn finish(
    env: &Env<'_>,
    request_id: &str,
    operation_id: &str,
    running: Running,
    outcome: Outcome,
) -> Value {
    let (decision, reply) = match &outcome.completion {
        Completion::Exited(0) => match parse_output(&running.decl, &outcome.stdout) {
            Ok(data) => (
                "ok",
                success(request_id, operation_id, data, outcome.stdout.len()),
            ),
            Err(message) => (
                "bad_output",
                failure(request_id, operation_id, code::BAD_OUTPUT, message),
            ),
        },
        Completion::Exited(_) | Completion::Signaled(_) if relocation_failure(&outcome.stderr) => (
            "not_self_contained",
            failure(
                request_id,
                operation_id,
                code::EXECUTABLE_NOT_SELF_CONTAINED,
                "the program loads libraries relative to its own location (@executable_path, @loader_path or @rpath). \
                 Operations run a verified private copy, so only self-contained programs are supported",
            ),
        ),
        Completion::Exited(status) => {
            let tail: String = outcome.stderr.chars().take(STDERR_IN_ERROR).collect();
            let mut reply = failure(
                request_id,
                operation_id,
                code::NONZERO_EXIT,
                format!("exited with status {status}: {}", tail.trim()),
            );
            reply["exit_code"] = json!(status);
            reply["stderr"] = json!(tail.trim());
            // Real CLIs report structured errors on stdout. It is held to the
            // same bound (enforced while reading) and the same format check as
            // a success; text that does not parse is never surfaced as data.
            match parse_output(&running.decl, &outcome.stdout) {
                Ok(data) if !outcome.stdout.is_empty() => {
                    reply["data"] = data;
                    reply["data_available"] = json!(true);
                }
                _ => reply["data_available"] = json!(false),
            }
            ("nonzero_exit", reply)
        }
        Completion::Signaled(signal) => (
            "signaled",
            failure(
                request_id,
                operation_id,
                code::LAUNCH_FAILED,
                format!("ended by signal {signal}"),
            ),
        ),
        Completion::TimedOut => (
            "timeout",
            failure(
                request_id,
                operation_id,
                code::TIMEOUT,
                format!("exceeded {} ms", running.decl.timeout_ms),
            ),
        ),
        Completion::OutputTooLarge => (
            "output_too_large",
            failure(
                request_id,
                operation_id,
                code::OUTPUT_TOO_LARGE,
                format!("output exceeded {} bytes", running.decl.output.max_bytes),
            ),
        ),
        Completion::Cancelled => (
            "cancelled",
            failure(request_id, operation_id, code::CANCELLED, "cancelled"),
        ),
    };
    if let Err(error) = env.monitor.note_outcome(
        &running.actor,
        &running.call_id,
        &running.grant_id,
        &running.fingerprint,
        &running.resource,
        "",
        decision,
        "",
        "",
    ) {
        log::error!("cli_ops: request={request_id} outcome audit failed: {error}");
    }
    reply
}

/// dyld's failure to find a library named relative to the program's own
/// location, which a copy in the private directory cannot satisfy.
fn relocation_failure(stderr: &str) -> bool {
    stderr.contains("Library not loaded")
        && ["@executable_path", "@loader_path", "@rpath"]
            .iter()
            .any(|marker| stderr.contains(marker))
}

fn parse_output(decl: &CliOperationDecl, stdout: &[u8]) -> Result<Value, String> {
    match decl.output.format {
        OutputFormat::Json => serde_json::from_slice::<Value>(stdout)
            .map_err(|e| format!("output is not valid JSON: {e}")),
        OutputFormat::Text => Ok(Value::String(String::from_utf8_lossy(stdout).into_owned())),
    }
}
