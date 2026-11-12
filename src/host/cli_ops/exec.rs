//! Executable identity, the sealed private copy, and the bounded supervisor.
//!
//! Race-resistant launch. macOS has no `fexecve`, and `execve("/dev/fd/N")`
//! fails with EACCES (measured on macOS 14.5, stint 1175 plan), so a verified
//! file descriptor cannot be executed directly. Instead:
//!
//! 1. The fixed executable is resolved and opened `O_NOFOLLOW`; the descriptor
//!    is `fstat`ed and hashed. Scripts, setuid/setgid and group- or
//!    world-writable files are refused.
//! 2. [`OpenedExecutable::seal`] streams the bytes from that descriptor into a
//!    fresh file in a host-owned `0700` directory while hashing the bytes it
//!    writes, requires the digest to equal the approved one, and makes the file
//!    `0500`. Replacing the original path afterwards cannot change what runs.
//! 3. [`SealedExecutable::verify`] reopens the sealed file `O_NOFOLLOW` right
//!    before spawn and re-checks inode and content digest.
//! 4. Only the sealed path is spawned. The user-visible path is never executed.
//!
//! Residual: a process running as the same user can still race inside the
//! random-named `0700` directory between step 3 and `exec`. That actor already
//! holds the user's full authority; the grant protects the Plexi plugin boundary,
//! not the account.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::app::cli_operation::hex;

/// Largest program the host will hash and copy per launch.
pub const MAX_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const READER_GRACE: Duration = Duration::from_millis(500);
const STDERR_KEEP: usize = 2048;

/// Why an executable cannot be used. Each variant maps to one guest-facing code.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("executable '{spec}' was not found in the fixed search directories ({searched})")]
    NotFound { spec: String, searched: String },
    #[error("executable {path} was refused: {reason}")]
    Refused { path: PathBuf, reason: String },
    #[error("executable content changed: approved sha256 {approved}, found {found} for {path}")]
    ContentMismatch {
        path: PathBuf,
        approved: String,
        found: String,
    },
    #[error("io error during {action} at {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io(action: &'static str, path: &Path, source: std::io::Error) -> ExecError {
    ExecError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

fn refused(path: &Path, reason: impl Into<String>) -> ExecError {
    ExecError::Refused {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// What an approval binds about the program: canonical path and content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIdentity {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// Fixed directories a bare executable name is looked up in. Only metadata is
/// read; nothing is executed while looking.
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs_home() {
        dirs.push(home.join(".cargo/bin"));
    }
    for fixed in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(fixed));
    }
    dirs
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Resolve the declared executable to a canonical file path. A missing or
/// non-regular file fails without probing any other program.
pub fn resolve(spec: &str, search_dirs: &[PathBuf]) -> Result<PathBuf, ExecError> {
    let candidates: Vec<PathBuf> = if spec.starts_with('/') {
        vec![PathBuf::from(spec)]
    } else {
        search_dirs.iter().map(|dir| dir.join(spec)).collect()
    };
    for candidate in &candidates {
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if canonical.is_file() {
            log::info!(
                "cli_ops: resolved executable spec={spec} -> {}",
                canonical.display()
            );
            return Ok(canonical);
        }
    }
    Err(ExecError::NotFound {
        spec: spec.to_string(),
        searched: if spec.starts_with('/') {
            spec.to_string()
        } else {
            search_dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        },
    })
}

/// An executable opened and hashed through one descriptor.
pub struct OpenedExecutable {
    file: File,
    pub identity: ExecutableIdentity,
}

fn open_nofollow(path: &Path) -> Result<File, ExecError> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| io("open", path, e))
}

/// Open `path` without following a final symlink, check it is a plain
/// executable regular file, and hash exactly the bytes behind the descriptor.
pub fn open_and_identify(path: &Path) -> Result<OpenedExecutable, ExecError> {
    let mut file = open_nofollow(path)?;
    let meta = file.metadata().map_err(|e| io("stat", path, e))?;
    if !meta.is_file() {
        return Err(refused(path, "not a regular file"));
    }
    let mode = meta.mode();
    if mode & 0o111 == 0 {
        return Err(refused(path, "not executable"));
    }
    if mode & 0o6000 != 0 {
        return Err(refused(path, "setuid or setgid programs are not admitted"));
    }
    if mode & 0o022 != 0 {
        return Err(refused(
            path,
            "writable by group or others; a program anyone can rewrite is not a stable identity",
        ));
    }
    if meta.len() == 0 || meta.len() > MAX_EXECUTABLE_BYTES {
        return Err(refused(
            path,
            format!("size {} is outside 1..={MAX_EXECUTABLE_BYTES}", meta.len()),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    let mut first = [0u8; 2];
    loop {
        let n = file.read(&mut buf).map_err(|e| io("read", path, e))?;
        if n == 0 {
            break;
        }
        if total == 0 {
            first[..n.min(2)].copy_from_slice(&buf[..n.min(2)]);
        }
        total += n as u64;
        if total > MAX_EXECUTABLE_BYTES {
            return Err(refused(path, "grew past the size limit while hashing"));
        }
        hasher.update(&buf[..n]);
    }
    if &first == b"#!" {
        return Err(refused(
            path,
            "script executables are not admitted: the interpreter identity would not be bound",
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io("seek", path, e))?;
    Ok(OpenedExecutable {
        file,
        identity: ExecutableIdentity {
            path: path.to_path_buf(),
            sha256: hex(&hasher.finalize()),
            size: total,
        },
    })
}

/// The private copy that is actually launched. Copies are cached by content
/// digest (`<base>/<program name>/<sha256>/program`), built once and reused; the
/// copy of a superseded digest of the same program name is removed when a new
/// digest is sealed.
pub struct SealedExecutable {
    path: PathBuf,
    sha256: String,
    dev: u64,
    ino: u64,
}

fn private_dir(path: &Path) -> Result<(), ExecError> {
    std::fs::create_dir_all(path).map_err(|e| io("create_dir", path, e))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| io("chmod", path, e))
}

/// Hash the file behind `file` from the start.
fn hash_fd(file: &mut File, path: &Path) -> Result<String, ExecError> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| io("read", path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

impl SealedExecutable {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A cached copy at `path`, only if its inode is a 0500 file owned by this
    /// user whose bytes hash to `sha256`.
    fn open_cached(path: &Path, sha256: &str) -> Option<Self> {
        let mut file = open_nofollow(path).ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file()
            || meta.mode() & 0o7777 != 0o500
            // SAFETY: plain syscall without arguments.
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return None;
        }
        (hash_fd(&mut file, path).ok()? == sha256).then(|| Self {
            path: path.to_path_buf(),
            sha256: sha256.to_string(),
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    /// Reopen the sealed file and require the same inode, ownership, mode and
    /// content digest. Called immediately before spawn.
    pub fn verify(&self) -> Result<(), ExecError> {
        let mut file = open_nofollow(&self.path)?;
        let meta = file.metadata().map_err(|e| io("stat", &self.path, e))?;
        if meta.dev() != self.dev || meta.ino() != self.ino {
            return Err(refused(&self.path, "sealed copy was replaced"));
        }
        // SAFETY: plain syscall without arguments.
        if meta.mode() & 0o7777 != 0o500 || meta.uid() != unsafe { libc::geteuid() } {
            return Err(refused(&self.path, "sealed copy mode or owner changed"));
        }
        let found = hash_fd(&mut file, &self.path)?;
        if found != self.sha256 {
            return Err(ExecError::ContentMismatch {
                path: self.path.clone(),
                approved: self.sha256.clone(),
                found,
            });
        }
        Ok(())
    }
}

impl OpenedExecutable {
    /// Make the bytes behind this descriptor available as a private copy under
    /// `base`, requiring them to hash to `approved_sha256`. A verified cached
    /// copy is reused. Otherwise the bytes are streamed from the descriptor
    /// into a temporary file while hashing what is written (not what was first
    /// read, which closes the window in which the original could change between
    /// the identity check and the copy) and linked into place only if the
    /// digest matches.
    pub fn seal(
        mut self,
        base: &Path,
        approved_sha256: &str,
    ) -> Result<SealedExecutable, ExecError> {
        if self.identity.sha256 != approved_sha256 {
            return Err(ExecError::ContentMismatch {
                path: self.identity.path.clone(),
                approved: approved_sha256.to_string(),
                found: self.identity.sha256.clone(),
            });
        }
        let name = self
            .identity
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| {
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            })
            .unwrap_or("program")
            .to_string();
        let name_dir = base.join(&name);
        let dir = name_dir.join(approved_sha256);
        let final_path = dir.join("program");
        private_dir(base)?;
        private_dir(&name_dir)?;
        private_dir(&dir)?;
        // A digest of this program that is no longer approved loses its copy.
        if let Ok(entries) = std::fs::read_dir(&name_dir) {
            for entry in entries.flatten() {
                if entry.file_name() != approved_sha256 {
                    if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                        log::warn!(
                            "cli_ops: could not remove superseded copy {}: {error}",
                            entry.path().display()
                        );
                    }
                }
            }
        }
        if let Some(hit) = SealedExecutable::open_cached(&final_path, approved_sha256) {
            log::info!("cli_ops: reusing sealed copy {}", final_path.display());
            return Ok(hit);
        }
        let tmp = dir.join(format!("program.tmp-{}", uuid::Uuid::new_v4().simple()));
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .map_err(|e| io("create", &tmp, e))?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| io("seek", &self.identity.path, e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut copied = 0u64;
        let built = (|| {
            loop {
                let n = self
                    .file
                    .read(&mut buf)
                    .map_err(|e| io("read", &self.identity.path, e))?;
                if n == 0 {
                    break;
                }
                copied += n as u64;
                if copied > MAX_EXECUTABLE_BYTES {
                    return Err(refused(
                        &self.identity.path,
                        "grew past the size limit while copying",
                    ));
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n]).map_err(|e| io("write", &tmp, e))?;
            }
            out.sync_all().map_err(|e| io("sync", &tmp, e))?;
            let found = hex(&hasher.finalize_reset());
            if found != approved_sha256 {
                return Err(ExecError::ContentMismatch {
                    path: self.identity.path.clone(),
                    approved: approved_sha256.to_string(),
                    found,
                });
            }
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o500))
                .map_err(|e| io("chmod", &tmp, e))
        })();
        drop(out);
        let linked = built.and_then(|()| {
            // A cached copy that failed verification is replaced, not trusted.
            let _ = std::fs::remove_file(&final_path);
            std::fs::hard_link(&tmp, &final_path).map_err(|e| io("link", &final_path, e))
        });
        let _ = std::fs::remove_file(&tmp);
        linked?;
        log::info!(
            "cli_ops: sealed {} ({} bytes, sha256 {}) -> {}",
            self.identity.path.display(),
            copied,
            &approved_sha256[..12],
            final_path.display()
        );
        SealedExecutable::open_cached(&final_path, approved_sha256).ok_or_else(|| {
            refused(
                &final_path,
                "sealed copy could not be verified after creation",
            )
        })
    }
}

/// Bounds the host enforces on one launch.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_output: usize,
}

/// Everything the host, never the guest, decides about one launch.
pub struct LaunchSpec {
    pub sealed: SealedExecutable,
    /// `argv[0]` shown to the program: the declared executable name.
    pub display_name: String,
    pub argv: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub limits: Limits,
}

/// The fixed, minimal environment of every launch.
pub fn fixed_env() -> Vec<(String, String)> {
    let mut env = vec![
        (
            "PATH".to_string(),
            "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
        ),
        ("LANG".to_string(), "en_US.UTF-8".to_string()),
    ];
    if let Some(home) = dirs_home() {
        env.push(("HOME".to_string(), home.display().to_string()));
    }
    env
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    OutputTooLarge,
    Cancelled,
}

#[derive(Debug)]
pub struct Outcome {
    pub completion: Completion,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub elapsed: Duration,
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// One launched operation. Dropping it cancels the process group and joins the
/// supervisor, so closing a plugin cannot leave its processes behind.
pub struct RunningOperation {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Outcome>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl RunningOperation {
    pub fn try_outcome(&self) -> Option<Outcome> {
        self.rx.try_recv().ok()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for RunningOperation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                log::error!("cli_ops: supervisor thread panicked during cleanup");
            }
        }
    }
}

fn kill_group(pid: i32) {
    // The child is its own process-group leader, so -pid reaches descendants.
    // SAFETY: plain syscall on a pid this module spawned.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

/// Verify the sealed copy and spawn it under supervision.
pub fn spawn_supervised(spec: LaunchSpec, wake: Wake) -> Result<RunningOperation, ExecError> {
    spec.sealed.verify()?;
    let path = spec.sealed.path().to_path_buf();
    let mut command = Command::new(&path);
    command
        .arg0(&spec.display_name)
        .args(&spec.argv)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|e| io("spawn", &path, e))?;
    let pid = child.id() as i32;
    log::info!(
        "cli_ops: launched pid={pid} program={} argc={} cwd={}",
        spec.display_name,
        spec.argv.len(),
        spec.cwd.display()
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = sync_channel::<Outcome>(1);
    let cancel_thread = Arc::clone(&cancel);
    let limits = spec.limits;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let sealed = spec.sealed;
    let join = std::thread::Builder::new()
        .name("cli-op".into())
        .spawn(move || {
            let started = Instant::now();
            let out_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let overflow = Arc::new(AtomicBool::new(false));
            let out_done = Arc::new(AtomicBool::new(false));
            let err_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let err_done = Arc::new(AtomicBool::new(false));
            spawn_reader(
                stdout_pipe,
                Arc::clone(&out_buf),
                Some((limits.max_output, Arc::clone(&overflow))),
                Arc::clone(&out_done),
            );
            spawn_reader(
                stderr_pipe,
                Arc::clone(&err_buf),
                None,
                Arc::clone(&err_done),
            );
            let deadline = started + limits.timeout;
            let mut forced: Option<Completion> = None;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => {}
                    Err(error) => {
                        log::error!("cli_ops: wait failed pid={pid}: {error}");
                        break None;
                    }
                }
                if cancel_thread.load(Ordering::SeqCst) {
                    forced = Some(Completion::Cancelled);
                } else if overflow.load(Ordering::SeqCst) {
                    forced = Some(Completion::OutputTooLarge);
                } else if Instant::now() >= deadline {
                    forced = Some(Completion::TimedOut);
                }
                if forced.is_some() {
                    kill_group(pid);
                    break child.wait().ok();
                }
                std::thread::sleep(POLL_INTERVAL);
            };
            if forced.is_none() && overflow.load(Ordering::SeqCst) {
                // The reader stopped at the cap and closed the pipe, so the
                // program may have exited on SIGPIPE before the loop saw it.
                forced = Some(Completion::OutputTooLarge);
            }
            // Whatever the leader did, nothing it started in this group survives.
            kill_group(pid);
            let grace = Instant::now() + READER_GRACE;
            while !(out_done.load(Ordering::SeqCst) && err_done.load(Ordering::SeqCst))
                && Instant::now() < grace
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            let completion = forced.unwrap_or_else(|| match status {
                Some(status) => {
                    use std::os::unix::process::ExitStatusExt;
                    match (status.code(), status.signal()) {
                        (Some(code), _) => Completion::Exited(code),
                        (None, Some(signal)) => Completion::Signaled(signal),
                        (None, None) => Completion::Exited(-1),
                    }
                }
                None => Completion::Exited(-1),
            });
            let stdout = std::mem::take(&mut *out_buf.lock().unwrap_or_else(|e| e.into_inner()));
            let stderr =
                String::from_utf8_lossy(&err_buf.lock().unwrap_or_else(|e| e.into_inner()))
                    .into_owned();
            let outcome = Outcome {
                completion,
                stdout,
                stderr,
                elapsed: started.elapsed(),
            };
            log::info!(
                "cli_ops: finished pid={pid} completion={:?} stdout_bytes={} elapsed_ms={}",
                outcome.completion,
                outcome.stdout.len(),
                outcome.elapsed.as_millis()
            );
            drop(sealed);
            if tx.send(outcome).is_err() {
                log::debug!("cli_ops: outcome dropped, operation was cancelled by its owner");
            }
            wake();
        })
        .map_err(|e| io("spawn_thread", &path, e))?;
    Ok(RunningOperation {
        cancel,
        rx,
        join: Some(join),
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    buf: Arc<Mutex<Vec<u8>>>,
    cap: Option<(usize, Arc<AtomicBool>)>,
    done: Arc<AtomicBool>,
) {
    let Some(mut pipe) = pipe else {
        done.store(true, Ordering::SeqCst);
        return;
    };
    let spawned = std::thread::Builder::new().name("cli-op-io".into()).spawn({
        let done = Arc::clone(&done);
        move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut guard = buf.lock().unwrap_or_else(|e| e.into_inner());
                        match &cap {
                            Some((max, overflow)) => {
                                if guard.len() + n > *max {
                                    overflow.store(true, Ordering::SeqCst);
                                    break;
                                }
                                guard.extend_from_slice(&chunk[..n]);
                            }
                            None => {
                                let room = STDERR_KEEP.saturating_sub(guard.len());
                                guard.extend_from_slice(&chunk[..n.min(room)]);
                            }
                        }
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
        }
    });
    if spawned.is_err() {
        done.store(true, Ordering::SeqCst);
    }
}
