//! The one way to run a helper process: bounded in time, never interactive,
//! and never Ok with incomplete work (stints 1605, 1610).
//!
//! `Helper` exposes only arguments, environment, working directory and the
//! bounded options below. stdin is `/dev/null` and output goes to temp files
//! unless a caller chooses otherwise; a pipe is drained by a thread whose wait
//! is itself bounded, so nothing a descendant keeps open can block the caller. On Unix the child leads a new
//! session with no controlling terminal, so it can neither read nor write the
//! caller's terminal; a session leader also leads its own process group.
//!
//! When the child exits, its exit is observed without reaping it, its group is
//! killed while the zombie still holds the group id, then it is reaped: a
//! background job it started dies with it. At the limit the group is killed
//! the same way. A process that called `setsid` itself, as the host does under
//! `host start`, is outside the group and survives. If a kill fails, or the
//! killed child is not reaped within `REAP_LIMIT`, the run still returns, with
//! `RunError::NotStopped`.
//!
//! Output past its policy's limit (`CAPTURE_LIMIT` by default) is
//! `RunError::OutputTooLarge`, never cut bytes; the size is checked while the
//! child runs, so a runaway writer is stopped rather than filling the disk
//! until its limit. Each stream's `OutputPolicy` can instead keep it in
//! memory only (secrets), keep a bounded tail (diagnostics) or discard it. A
//! caller may cancel a run (`cancel_on`) or feed it stdin (`input`).
//!
//! Callers: a helper that changes state (an install, a venv, a git checkout,
//! a copy) fails its caller on any error except `RunError::is_tool_failure`,
//! and only where the tool is optional. A read-only probe (a version, a
//! path, `--help`, hardware facts) may degrade to "unknown" on any error,
//! since it changed nothing. LaunchServices registration after an install
//! has committed is the one state-changing helper that degrades; see
//! `transaction::lsregister`.
//! Windows kills only the child; descendants there are stint 1613.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

/// The most read back from each output stream. More than this is an error.
pub const CAPTURE_LIMIT: u64 = 1024 * 1024;

/// How long a killed child gets to be reaped before the run gives up on it.
const REAP_LIMIT: Duration = Duration::from_secs(5);

/// The longest pause between exit checks. Checks start at 1 ms and back off
/// to this, so a fast helper does not pay a fixed delay.
const MAX_POLL: Duration = Duration::from_millis(20);

/// Both output streams of a helper, each read whole.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A helper that exited by itself inside its limit, with any exit code.
#[derive(Debug)]
pub struct Exited {
    pub status: ExitStatus,
    pub output: Output,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not start {what}: {source}")]
    Start {
        what: String,
        source: std::io::Error,
    },
    #[error("{what} failed ({status}){}", stderr_suffix(stderr))]
    Failed {
        what: String,
        status: ExitStatus,
        stderr: Vec<u8>,
    },
    #[error("{what} did not finish within {limit:?} and was stopped")]
    TimedOut {
        what: String,
        limit: Duration,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    #[error("{what} was cancelled and stopped")]
    Cancelled { what: String },
    #[error("{what} could not be stopped ({cause}); it may still be running")]
    NotStopped { what: String, cause: String },
    #[error("{what} wrote more than {limit} bytes to {stream}; its output is incomplete")]
    OutputTooLarge {
        what: String,
        stream: &'static str,
        limit: u64,
    },
    #[error("{action} for {what}: {source}")]
    Io {
        what: String,
        action: &'static str,
        source: std::io::Error,
    },
}

impl RunError {
    /// The tool would not start or exited non-zero: it said no, and nothing
    /// of it is left running or was cut short. A caller for which the tool
    /// is optional may degrade on this. Every other error (a timeout, a run
    /// that could not be stopped, cut output, an I/O failure) means the work
    /// is in an unknown state, and a caller that changes state must fail.
    pub fn is_tool_failure(&self) -> bool {
        matches!(self, Self::Start { .. } | Self::Failed { .. })
    }
}

fn stderr_suffix(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

/// What happens to one output stream of a helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPolicy {
    /// Captured to a temp file and read back whole. Past `limit` bytes the
    /// child is stopped and the run is `RunError::OutputTooLarge`. The
    /// default for both streams, at `CAPTURE_LIMIT`.
    File { limit: u64 },
    /// Captured in memory only, never on disk: a thread drains a pipe into a
    /// buffer allocated up front, so it is never reallocated and copied, and
    /// zeroes its read buffer. Past `limit` bytes the child is stopped and
    /// the run is `RunError::OutputTooLarge`. For secrets; the caller zeroes
    /// what it gets back. A process outside the session that keeps the pipe
    /// open fails the run rather than holding it.
    Memory { limit: usize },
    /// Drained continuously; only the last `keep` bytes are kept. Never fails
    /// the run, whatever the stream's size. For diagnostics.
    Tail { keep: usize },
    /// Sent to the null device. For output no one reads.
    Discard,
}

/// How long a pipe's drain or input thread gets to finish after the child's
/// session has ended. Only a process outside the session can hold it longer.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// A helper process to run once, bounded. Built like a `Command`, but only
/// arguments, environment, working directory and the bounded options below
/// can be set.
pub struct Helper {
    command: Command,
    stdout: OutputPolicy,
    stderr: OutputPolicy,
    cancel: Option<Arc<AtomicBool>>,
    input: Option<Zeroizing<Vec<u8>>>,
}

impl Helper {
    #[expect(
        clippy::disallowed_methods,
        reason = "helper: the one bounded launch every helper goes through"
    )]
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: Command::new(program),
            stdout: OutputPolicy::File {
                limit: CAPTURE_LIMIT,
            },
            stderr: OutputPolicy::File {
                limit: CAPTURE_LIMIT,
            },
            cancel: None,
            input: None,
        }
    }

    /// The program's `argv[0]`, when it should differ from the path run.
    #[cfg(unix)]
    pub fn arg0(&mut self, arg0: impl AsRef<OsStr>) -> &mut Self {
        std::os::unix::process::CommandExt::arg0(&mut self.command, arg0);
        self
    }

    /// How stdout is captured; `OutputPolicy::File` at `CAPTURE_LIMIT` unless
    /// set.
    pub fn stdout(&mut self, policy: OutputPolicy) -> &mut Self {
        self.stdout = policy;
        self
    }

    /// How stderr is captured; `OutputPolicy::File` at `CAPTURE_LIMIT` unless
    /// set.
    pub fn stderr(&mut self, policy: OutputPolicy) -> &mut Self {
        self.stderr = policy;
        self
    }

    /// Once `flag` is set the child and its session are stopped and the run
    /// is `RunError::Cancelled`. Checked at every exit check, so within
    /// `MAX_POLL`.
    pub fn cancel_on(&mut self, flag: Arc<AtomicBool>) -> &mut Self {
        self.cancel = Some(flag);
        self
    }

    /// stdin is a pipe a writer thread feeds `bytes` into, then closes. Any
    /// size works; a child that never reads holds only that thread, and the
    /// run still ends at its deadline, when the session's death breaks the
    /// pipe. The bytes are zeroed once written. A write that fails for any
    /// reason but the child closing stdin fails the run.
    pub fn input(&mut self, bytes: impl Into<Vec<u8>>) -> &mut Self {
        self.input = Some(Zeroizing::new(bytes.into()));
        self
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.command.env_clear();
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }

    /// The environment changes set so far, as `Command::get_envs` reports
    /// them. For tests of a constructor's policy.
    pub fn get_envs(&self) -> std::process::CommandEnvs<'_> {
        self.command.get_envs()
    }

    /// Run to completion within `limit`. Ok only for exit 0 with both streams
    /// read whole; a non-zero exit is `RunError::Failed`.
    pub fn run(&mut self, what: &str, limit: Duration) -> Result<Output, RunError> {
        let exited = self.run_any_exit(what, limit)?;
        if !exited.status.success() {
            return Err(RunError::Failed {
                what: what.to_string(),
                status: exited.status,
                stderr: exited.output.stderr,
            });
        }
        Ok(exited.output)
    }

    /// As `run`, but any exit code is Ok, for callers that read the code.
    pub fn run_any_exit(&mut self, what: &str, limit: Duration) -> Result<Exited, RunError> {
        self.start(what)?.wait(limit)
    }

    /// Start the child and return at once, for a caller that waits for it on
    /// another thread. The limit `Started::wait` is given counts from here.
    pub fn start(&mut self, what: &str) -> Result<Started, RunError> {
        let io = |action: &'static str| {
            move |source| RunError::Io {
                what: what.to_string(),
                action,
                source,
            }
        };
        let (stdout, stdout_stdio) = Sink::open(self.stdout).map_err(io("open stdout"))?;
        let (stderr, stderr_stdio) = Sink::open(self.stderr).map_err(io("open stderr"))?;
        let (stdin_stdio, input_writer) = match self.input.take() {
            Some(bytes) => {
                let (reader, writer) = std::io::pipe().map_err(io("open stdin"))?;
                (Stdio::from(reader), Some((writer, bytes)))
            }
            None => (Stdio::null(), None),
        };
        self.command
            .stdin(stdin_stdio)
            .stdout(stdout_stdio)
            .stderr(stderr_stdio);
        // SAFETY: between fork and exec the closure calls only setsid(), which
        // is async-signal-safe and neither allocates nor locks.
        #[cfg(unix)]
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut self.command, || {
                // A forked child never leads a process group, so setsid
                // succeeds.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = self.command.spawn();
        let started_at = Instant::now();
        // Close the parent's copies of the child's pipe ends, so a drain sees
        // end of file once the child's session is gone.
        self.command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawned.map_err(|source| RunError::Start {
            what: what.to_string(),
            source,
        })?;
        let mut started = Started {
            child: Some(child),
            what: what.to_string(),
            started_at,
            stdout,
            stderr,
            input: None,
            cancel: self.cancel.clone(),
        };
        if let Some((writer, bytes)) = input_writer {
            // On failure `started` drops here, which stops the child.
            started.input = Some(Delivery::spawn(writer, bytes).map_err(io("start input"))?);
        }
        Ok(started)
    }
}

/// stdin bytes being written by their own thread. On Unix the write end is
/// non-blocking and written in a poll loop that also watches `stop`, so a
/// reader that never reads (even a descendant that left the session holding
/// stdin) cannot pin the thread, the pipe or the bytes: once the run ends,
/// `stop` is set and the thread exits within `INPUT_POLL`, dropping (and so
/// zeroing) the bytes on every path. Windows is not covered yet; see
/// `write_until_stopped` there.
struct Delivery {
    done: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    result: Arc<Mutex<Option<std::io::Result<()>>>>,
}

/// The longest an input thread waits for room in the pipe before it looks at
/// `stop` again.
#[cfg(unix)]
const INPUT_POLL: Duration = Duration::from_millis(10);

impl Delivery {
    fn spawn(writer: std::io::PipeWriter, bytes: Zeroizing<Vec<u8>>) -> std::io::Result<Self> {
        #[cfg(unix)]
        set_nonblocking(&writer)?;
        let done = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let (thread_done, thread_stop, thread_result) =
            (Arc::clone(&done), Arc::clone(&stop), Arc::clone(&result));
        std::thread::Builder::new()
            .name("helper-input".into())
            .spawn(move || {
                let written = write_until_stopped(writer, &bytes, &thread_stop);
                drop(bytes);
                *thread_result.lock().unwrap_or_else(|e| e.into_inner()) = Some(written);
                thread_done.store(true, Ordering::SeqCst);
            })?;
        Ok(Self { done, stop, result })
    }

    /// After the child's session has ended: Ok if every byte was written or
    /// the child closed stdin itself (its exit says whether that mattered).
    fn finish(&self, what: &str) -> Result<(), RunError> {
        let io = |source| RunError::Io {
            what: what.to_string(),
            action: "write stdin",
            source,
        };
        if !wait_flag(&self.done, DRAIN_LIMIT) {
            self.stop.store(true, Ordering::SeqCst);
            return Err(io(std::io::Error::other(
                "a process outside its session still holds stdin open without reading it",
            )));
        }
        match self.result.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(Err(error)) if error.kind() != std::io::ErrorKind::BrokenPipe => Err(io(error)),
            _ => Ok(()),
        }
    }
}

impl Drop for Delivery {
    /// However the run ended, the input thread stops writing and exits (on
    /// Unix; Windows ignores `stop`, see `write_until_stopped`).
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(unix)]
fn set_nonblocking(writer: &std::io::PipeWriter) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = writer.as_raw_fd();
    // SAFETY: fcntl reads and sets the flags of a pipe fd the caller owns.
    let set = unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    };
    if set {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Write all of `bytes`, waiting for room at most `INPUT_POLL` at a time,
/// until done or `stop` is set. Stopped early is `Interrupted`.
#[cfg(unix)]
fn write_until_stopped(
    mut writer: std::io::PipeWriter,
    bytes: &[u8],
    stop: &AtomicBool,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut rest = bytes;
    while !rest.is_empty() {
        if stop.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "the run ended before stdin was written",
            ));
        }
        match writer.write(rest) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => rest = &rest[n..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let mut ready = libc::pollfd {
                    fd: writer.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                // SAFETY: poll reads one pollfd naming a pipe fd this thread
                // owns. Its result only decides when to try the write again.
                unsafe { libc::poll(&mut ready, 1, INPUT_POLL.as_millis() as libc::c_int) };
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Windows pipes here are blocking and `stop` is not watched. Killing the
/// child at its deadline breaks the pipe only if nothing else holds stdin: a
/// surviving descendant that keeps stdin open without reading (Windows kills
/// only the child), or a kill that fails, can keep this thread and its
/// buffer alive. Tracked by stint 1613.
#[cfg(windows)]
fn write_until_stopped(
    mut writer: std::io::PipeWriter,
    bytes: &[u8],
    _stop: &AtomicBool,
) -> std::io::Result<()> {
    writer.write_all(bytes)
}

/// Whether `flag` was set within `limit`.
fn wait_flag(flag: &AtomicBool, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while !flag.load(Ordering::SeqCst) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

/// A helper that has been started and must be waited for. Dropping it
/// without `wait` stops the child and its session, so a holder that cannot
/// wait (a supervisor thread that failed to start) leaves nothing running.
#[must_use = "a started helper is stopped when dropped; call wait"]
pub struct Started {
    child: Option<Child>,
    what: String,
    started_at: Instant,
    stdout: Sink,
    stderr: Sink,
    input: Option<Delivery>,
    cancel: Option<Arc<AtomicBool>>,
}

/// How long a started helper may run before it is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deadline {
    /// Stopped and `RunError::TimedOut` once this long has passed since the
    /// start. Every helper the host waits on for an answer uses this.
    After(Duration),
    /// No wall-clock limit: the child belongs to the host session, like a
    /// pane, and runs until it exits, is cancelled, outgrows its capture, or
    /// the host ends the session. Only for work the user started and waits
    /// for off the host loop (a saved command's shell step); the caller must
    /// register it with host quit cleanup, since this run never stops it on
    /// its own.
    HostSession,
}

impl Started {
    /// The child's process id (and, on Unix, its session and group id).
    pub fn id(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }

    /// Wait until `limit` after the start. As `Helper::run_any_exit`.
    pub fn wait(self, limit: Duration) -> Result<Exited, RunError> {
        self.wait_for(Deadline::After(limit))
    }

    /// Wait under `deadline`, counted from the start.
    pub fn wait_for(self, deadline: Deadline) -> Result<Exited, RunError> {
        self.wait_with(deadline, exited_unreaped)
    }

    /// `wait_for` with the exit check injected, so a test can make it fail.
    fn wait_with(
        mut self,
        deadline: Deadline,
        exited: fn(&mut Child) -> std::io::Result<bool>,
    ) -> Result<Exited, RunError> {
        let Some(child) = self.child.take() else {
            return Err(RunError::NotStopped {
                what: self.what.clone(),
                cause: "its child was already given up".to_string(),
            });
        };
        let status = supervise(
            child,
            &self.what,
            match deadline {
                Deadline::After(limit) => Some((self.started_at + limit, limit)),
                Deadline::HostSession => None,
            },
            [("stdout", &self.stdout), ("stderr", &self.stderr)],
            self.cancel.as_deref(),
            exited,
        )?;
        if let Some(input) = &self.input {
            input.finish(&self.what)?;
        }
        let stdout = std::mem::replace(&mut self.stdout, Sink::Discard);
        let stderr = std::mem::replace(&mut self.stderr, Sink::Discard);
        let output = Output {
            stdout: stdout.finish(&self.what, "stdout")?,
            stderr: stderr.finish(&self.what, "stderr")?,
        };
        Ok(Exited { status, output })
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            // `stop` logs the outcome; there is no caller left to tell.
            let _ = stop(child, &self.what, Stopped::Cancelled, kill_tree, REAP_LIMIT);
        }
    }
}

/// Wait for the child to exit before `deadline` (the instant and the limit
/// it came from; none for `Deadline::HostSession`), stopping it there, on
/// cancel, when a capture outgrows its limit, or when the exit check itself
/// fails. Every error path stops and reaps the session before returning.
fn supervise(
    mut child: Child,
    what: &str,
    deadline: Option<(Instant, Duration)>,
    sinks: [(&'static str, &Sink); 2],
    cancel: Option<&AtomicBool>,
    exited_check: fn(&mut Child) -> std::io::Result<bool>,
) -> Result<ExitStatus, RunError> {
    let mut pause = Duration::from_millis(1);
    loop {
        let exited = match exited_check(&mut child) {
            Ok(exited) => exited,
            Err(error) => {
                return Err(stop(
                    child,
                    what,
                    Stopped::WaitFailed(error),
                    kill_tree,
                    REAP_LIMIT,
                ));
            }
        };
        let reason = match next_step(Instant::now(), deadline.map(|(at, _)| at), exited, pause) {
            Step::Finish => return finish_session(child, what),
            Step::Stop => Stopped::TimedOut {
                limit: deadline.map_or(Duration::ZERO, |(_, limit)| limit),
                stdout: sinks[0].1.partial(),
                stderr: sinks[1].1.partial(),
            },
            Step::Wait(wait) => {
                if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    Stopped::Cancelled
                } else if let Some((stream, limit)) = sinks
                    .iter()
                    .find_map(|(stream, sink)| sink.over_limit().map(|limit| (*stream, limit)))
                {
                    Stopped::OutputTooLarge { stream, limit }
                } else {
                    std::thread::sleep(wait);
                    pause = (pause * 2).min(MAX_POLL);
                    continue;
                }
            }
        };
        return Err(stop(child, what, reason, kill_tree, REAP_LIMIT));
    }
}

/// Where one output stream goes while the child runs.
enum Sink {
    File {
        file: tempfile::NamedTempFile,
        limit: u64,
    },
    Pipe {
        drained: Arc<Drained>,
        /// `Some` for `OutputPolicy::Memory` (fatal past it), `None` for
        /// `OutputPolicy::Tail`.
        memory_limit: Option<usize>,
    },
    Discard,
}

/// What a drain thread has read from a pipe.
struct Drained {
    bytes: Mutex<Zeroizing<Vec<u8>>>,
    over: AtomicBool,
    failed: Mutex<Option<std::io::Error>>,
    done: AtomicBool,
}

impl Sink {
    /// The sink for `policy`, and the child's end of it.
    fn open(policy: OutputPolicy) -> std::io::Result<(Self, Stdio)> {
        match policy {
            OutputPolicy::File { limit } => {
                // Named so the read-back opens its own handle: a positioned
                // read on the child's handle would share (on Windows, move)
                // the writer's offset.
                let file = tempfile::NamedTempFile::new()?;
                let stdio = Stdio::from(file.as_file().try_clone()?);
                Ok((Self::File { file, limit }, stdio))
            }
            OutputPolicy::Memory { limit } => Self::pipe(Some(limit), limit),
            OutputPolicy::Tail { keep } => Self::pipe(None, keep),
            OutputPolicy::Discard => Ok((Self::Discard, Stdio::null())),
        }
    }

    fn pipe(memory_limit: Option<usize>, size: usize) -> std::io::Result<(Self, Stdio)> {
        const CHUNK: usize = 8 * 1024;
        let (mut reader, writer) = std::io::pipe()?;
        // Room for every byte kept plus one chunk, so the buffer never
        // reallocates (which would leave a copy behind).
        let drained = Arc::new(Drained {
            bytes: Mutex::new(Zeroizing::new(Vec::with_capacity(size + CHUNK))),
            over: AtomicBool::new(false),
            failed: Mutex::new(None),
            done: AtomicBool::new(false),
        });
        let thread = Arc::clone(&drained);
        std::thread::Builder::new()
            .name("helper-drain".into())
            .spawn(move || {
                let mut chunk = Zeroizing::new([0u8; CHUNK]);
                loop {
                    match reader.read(&mut chunk[..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut bytes = thread.bytes.lock().unwrap_or_else(|e| e.into_inner());
                            if memory_limit.is_some_and(|limit| bytes.len() + n > limit) {
                                // Closing the pipe stops a writer that keeps
                                // going; the supervisor stops the child.
                                thread.over.store(true, Ordering::SeqCst);
                                break;
                            }
                            bytes.extend_from_slice(&chunk[..n]);
                            if memory_limit.is_none() && bytes.len() > size {
                                let cut = bytes.len() - size;
                                bytes.drain(..cut);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            *thread.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                            break;
                        }
                    }
                }
                thread.done.store(true, Ordering::SeqCst);
            })?;
        Ok((
            Self::Pipe {
                drained,
                memory_limit,
            },
            Stdio::from(writer),
        ))
    }

    /// The limit this sink has already passed while the child runs. An
    /// unreadable file size is not an overflow; the read after exit reports
    /// it.
    fn over_limit(&self) -> Option<u64> {
        match self {
            Self::File { file, limit } => file
                .as_file()
                .metadata()
                .is_ok_and(|meta| meta.len() > *limit)
                .then_some(*limit),
            Self::Pipe {
                drained,
                memory_limit: Some(limit),
            } => drained.over.load(Ordering::SeqCst).then_some(*limit as u64),
            Self::Pipe { .. } | Self::Discard => None,
        }
    }

    /// What was written so far, for a timeout's error. Memory captures may
    /// be secrets and are never copied into an error.
    fn partial(&self) -> Vec<u8> {
        match self {
            Self::File { file, limit } => read_capped(file.path(), *limit)
                .map(|(bytes, _)| bytes)
                .unwrap_or_default(),
            Self::Pipe {
                drained,
                memory_limit: None,
            } => drained
                .bytes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .to_vec(),
            Self::Pipe { .. } | Self::Discard => Vec::new(),
        }
    }

    /// The whole stream, once the child's session has ended.
    fn finish(self, what: &str, stream: &'static str) -> Result<Vec<u8>, RunError> {
        let io = |action: &'static str, source| RunError::Io {
            what: what.to_string(),
            action,
            source,
        };
        let too_large = |limit: u64| RunError::OutputTooLarge {
            what: what.to_string(),
            stream,
            limit,
        };
        match self {
            Self::File { file, limit } => {
                let (bytes, over) =
                    read_capped(file.path(), limit).map_err(|e| io("read output", e))?;
                if over {
                    return Err(too_large(limit));
                }
                Ok(bytes)
            }
            Self::Pipe {
                drained,
                memory_limit,
            } => {
                let finished = wait_flag(&drained.done, DRAIN_LIMIT);
                let take = || {
                    std::mem::take(&mut **drained.bytes.lock().unwrap_or_else(|e| e.into_inner()))
                };
                let Some(limit) = memory_limit else {
                    if !finished {
                        log::warn!(
                            "distribution: {what} {stream} is still held open outside its session; keeping the tail read so far"
                        );
                    }
                    return Ok(take());
                };
                if !finished {
                    return Err(io(
                        "drain output",
                        std::io::Error::other(format!(
                            "a process outside its session still holds {stream} open"
                        )),
                    ));
                }
                if drained.over.load(Ordering::SeqCst) {
                    return Err(too_large(limit as u64));
                }
                if let Some(error) = drained
                    .failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    return Err(io("drain output", error));
                }
                Ok(take())
            }
            Self::Discard => Ok(Vec::new()),
        }
    }
}

/// A `git` helper that never prompts, and the only way any module starts
/// git. Besides the helper's missing terminal, every other prompt route is
/// shut: terminal prompts (`GIT_TERMINAL_PROMPT=0`), askpass programs (an
/// empty `GIT_ASKPASS` stops git looking further), and ssh's `SSH_ASKPASS`
/// (cleared and refused). Credentials must come from an ssh key or agent or
/// a credential helper; `git_error` says so when they are missing.
pub fn git() -> Helper {
    git_at("git")
}

/// As `git`, for the git at `program` (a caller's injected fake in tests).
pub fn git_at(program: impl AsRef<OsStr>) -> Helper {
    let mut helper = Helper::new(program);
    helper
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env_remove("SSH_ASKPASS");
    helper
}

/// A git helper's error, plus how to set up credentials when git failed for
/// want of ones it was not allowed to prompt for.
pub fn git_error(error: &RunError) -> String {
    const AUTH_MARKERS: &[&str] = &[
        "terminal prompts disabled",
        "could not read Username",
        "could not read Password",
        "Authentication failed",
        "Permission denied (publickey",
        "Host key verification failed",
        "Could not read from remote repository",
    ];
    let mut message = error.to_string();
    if let RunError::Failed { stderr, .. } = error {
        let stderr = String::from_utf8_lossy(stderr);
        if AUTH_MARKERS.iter().any(|marker| stderr.contains(marker)) {
            message.push_str(
                "\ngit could not authenticate and Plexi does not prompt for credentials. \
For an ssh URL, add an ssh key for the host to your ssh agent and accept its host key \
once with `ssh -T <host>`; for an https GitHub URL, run `gh auth login` then \
`gh auth setup-git`. Then retry.",
            );
        }
    }
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The child exited inside its limit.
    Finish,
    /// The limit has passed, whether or not the child has exited since: an
    /// exit seen after the limit is not a success.
    Stop,
    /// Check again after this long, never past the limit.
    Wait(Duration),
}

fn next_step(now: Instant, deadline: Option<Instant>, exited: bool, pause: Duration) -> Step {
    match deadline {
        Some(deadline) if now >= deadline => Step::Stop,
        _ if exited => Step::Finish,
        Some(deadline) => Step::Wait(pause.min(deadline - now)),
        None => Step::Wait(pause),
    }
}

/// Whether the child has exited, without reaping it, so its pid (and group
/// id) stay reserved until the group is killed.
#[cfg(unix)]
fn exited_unreaped(child: &mut Child) -> std::io::Result<bool> {
    // SAFETY: a zeroed siginfo_t is a valid out-parameter for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: waitid writes only into `info`; WNOWAIT leaves the child
    // unreaped, and the pid names our own unreaped child.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    // A child that has not exited leaves si_pid zero.
    #[cfg(target_vendor = "apple")]
    let pid = info.si_pid;
    // SAFETY: waitid filled `info` for a child state change.
    #[cfg(not(target_vendor = "apple"))]
    let pid = unsafe { info.si_pid() };
    Ok(pid != 0)
}

#[cfg(windows)]
fn exited_unreaped(child: &mut Child) -> std::io::Result<bool> {
    child.try_wait().map(|status| status.is_some())
}

/// The child exited by itself: kill what is left of its session, then reap
/// it.
fn finish_session(mut child: Child, what: &str) -> Result<ExitStatus, RunError> {
    let killed = kill_tree(&mut child);
    let status = child.wait().map_err(|source| RunError::Io {
        what: what.to_string(),
        action: "reap",
        source,
    })?;
    if let Err(error) = killed {
        return Err(RunError::NotStopped {
            what: what.to_string(),
            cause: format!("it exited ({status}) but its session could not be ended: {error}"),
        });
    }
    Ok(status)
}

/// How long the processes left in a helper's session get to die.
#[cfg(unix)]
const SESSION_KILL_LIMIT: Duration = Duration::from_secs(2);

/// End the child's session: kill its process group, then repeatedly kill
/// every process group that still has a live member in the session and take
/// a fresh inventory, until two inventories in a row find nothing. The child
/// leads the session, so its pid is the session id; it is unreaped, so the
/// id cannot be reused. A process that called setsid itself has left the
/// session and is not touched.
///
/// macOS has no atomic "kill this session" (no cgroups, no pidfd), so this
/// converges rather than proves. Helpers are our own tools (git, uv, python,
/// sh), not hostile code: a descendant that deliberately moves to a new
/// process group and races fork against exit between two inventories can
/// still escape. That residual risk is accepted; helpers are not a sandbox.
/// Not converging within `SESSION_KILL_LIMIT`, or an inventory that cannot
/// be read, is an error.
#[cfg(unix)]
fn kill_tree(child: &mut Child) -> std::io::Result<()> {
    end_session(child.id() as libc::pid_t, all_pids)
}

#[cfg(unix)]
fn end_session(
    session: libc::pid_t,
    inventory: fn() -> std::io::Result<Vec<libc::pid_t>>,
) -> std::io::Result<()> {
    // SAFETY: kill and killpg only send signals; the pid names our unreaped
    // child. macOS refuses killpg with EPERM once only the exited leader is
    // left, which the inventory below then confirms.
    unsafe {
        libc::kill(session, libc::SIGKILL);
        libc::killpg(session, libc::SIGKILL);
    }
    let deadline = Instant::now() + SESSION_KILL_LIMIT;
    let mut empty_in_a_row = 0;
    loop {
        let alive = live_session_members(session, inventory()?);
        if alive.is_empty() {
            empty_in_a_row += 1;
            if empty_in_a_row == 2 {
                return Ok(());
            }
        } else {
            empty_in_a_row = 0;
            for &pid in &alive {
                // SAFETY: getpgid reads a group id; a group never spans
                // sessions, so killing a member's group touches only the
                // session. kill covers a member that changed group since.
                unsafe {
                    let group = libc::getpgid(pid);
                    if group > 0 {
                        libc::killpg(group, libc::SIGKILL);
                    }
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(format!(
                "its session did not empty within {SESSION_KILL_LIMIT:?}; still alive: {alive:?}"
            )));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Live processes in `pids` whose session is `session`, other than its
/// leader. Exited processes waiting to be reaped are not alive.
#[cfg(unix)]
fn live_session_members(session: libc::pid_t, pids: Vec<libc::pid_t>) -> Vec<libc::pid_t> {
    pids.into_iter()
        // SAFETY: getsid only reads the session id of a pid.
        .filter(|&pid| pid != session && unsafe { libc::getsid(pid) } == session)
        .filter(|&pid| !is_zombie(pid))
        .collect()
}

#[cfg(target_os = "macos")]
fn all_pids() -> std::io::Result<Vec<libc::pid_t>> {
    // SAFETY: proc_listallpids writes at most `size` bytes into `buffer`.
    all_pids_with(|buffer, size| unsafe { libc::proc_listallpids(buffer, size) })
}

/// Every pid, from `list` with `proc_listallpids`'s contract: a null buffer
/// asks for the count; otherwise it fills the buffer and returns how many
/// pids it wrote. Apple's implementation returns 0 when enumeration fails
/// (xnu libsyscall/wrappers/libproc/libproc.c), and there is always at
/// least this process, so 0 or less is an error, never an empty system.
#[cfg(target_os = "macos")]
fn all_pids_with(
    list: impl Fn(*mut libc::c_void, libc::c_int) -> libc::c_int,
) -> std::io::Result<Vec<libc::pid_t>> {
    let failed = |call: &str, result: libc::c_int| {
        std::io::Error::other(format!(
            "proc_listallpids {call} returned {result}: {}",
            std::io::Error::last_os_error()
        ))
    };
    let count = list(std::ptr::null_mut(), 0);
    if count <= 0 {
        return Err(failed("count", count));
    }
    // Room for processes started between the two calls.
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let written = list(
        pids.as_mut_ptr().cast(),
        std::mem::size_of_val(pids.as_slice()) as libc::c_int,
    );
    if written <= 0 {
        return Err(failed("list", written));
    }
    pids.truncate(written as usize);
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn is_zombie(pid: libc::pid_t) -> bool {
    // SAFETY: a zeroed proc_bsdinfo is a valid out-parameter.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into `info`.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    // Unreadable counts as alive, so the caller looks again rather than
    // declaring the session empty.
    written == size && info.pbi_status == libc::SZOMB
}

#[cfg(not(target_os = "macos"))]
#[cfg(unix)]
fn all_pids() -> std::io::Result<Vec<libc::pid_t>> {
    let pids: Vec<libc::pid_t> = std::fs::read_dir("/proc")?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    // There is always at least this process; none means /proc is unusable.
    if pids.is_empty() {
        return Err(std::io::Error::other("/proc lists no processes"));
    }
    Ok(pids)
}

#[cfg(not(target_os = "macos"))]
#[cfg(unix)]
fn is_zombie(pid: libc::pid_t) -> bool {
    // The state follows the last ')' of /proc/<pid>/stat, past the command.
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    })
}

/// Windows kills only the child; capture through files means a surviving
/// descendant cannot block the caller.
#[cfg(windows)]
fn kill_tree(child: &mut Child) -> std::io::Result<()> {
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        _ => child.kill(),
    }
}

/// Why a running child is being stopped.
#[derive(Debug)]
enum Stopped {
    TimedOut {
        limit: Duration,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Cancelled,
    OutputTooLarge {
        stream: &'static str,
        limit: u64,
    },
    /// Checking whether the child had exited failed.
    WaitFailed(std::io::Error),
}

impl Stopped {
    fn describe(&self) -> String {
        match self {
            Self::TimedOut { limit, .. } => format!("it exceeded {limit:?}"),
            Self::Cancelled => "it was cancelled".to_string(),
            Self::OutputTooLarge { stream, limit } => {
                format!("it wrote more than {limit} bytes to {stream}")
            }
            Self::WaitFailed(error) => format!("checking whether it exited failed: {error}"),
        }
    }

    fn into_error(self, what: &str) -> RunError {
        let what = what.to_string();
        match self {
            Self::TimedOut {
                limit,
                stdout,
                stderr,
            } => RunError::TimedOut {
                what,
                limit,
                stdout,
                stderr,
            },
            Self::Cancelled => RunError::Cancelled { what },
            Self::OutputTooLarge { stream, limit } => RunError::OutputTooLarge {
                what,
                stream,
                limit,
            },
            Self::WaitFailed(source) => RunError::Io {
                what,
                action: "wait",
                source,
            },
        }
    }
}

/// Stop a running child: kill it with `kill`, then reap it, waiting at most
/// `reap_limit` so a failed kill never holds the caller. Only a kill that
/// succeeded followed by a reap is the error `reason` names; anything else is
/// `NotStopped`.
fn stop(
    mut child: Child,
    what: &str,
    reason: Stopped,
    kill: fn(&mut Child) -> std::io::Result<()>,
    reap_limit: Duration,
) -> RunError {
    let killed = kill(&mut child);
    let reap_deadline = Instant::now() + reap_limit;
    let reaped = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < reap_deadline => std::thread::sleep(MAX_POLL),
            Ok(None) => break Err(format!("still running {reap_limit:?} after the kill")),
            Err(error) => break Err(format!("wait failed: {error}")),
        }
    };
    let why = reason.describe();
    let cause = match (killed, reaped) {
        (Ok(()), Ok(status)) => {
            log::warn!("distribution: {what} stopped because {why}; killed and reaped ({status})");
            return reason.into_error(what);
        }
        (Err(error), Ok(status)) => {
            format!("killing it failed: {error}; it then exited ({status})")
        }
        (Err(error), Err(reap)) => format!("killing it failed: {error}; {reap}"),
        (Ok(()), Err(reap)) => reap,
    };
    log::error!("distribution: {what} could not be stopped after {why}: {cause}");
    RunError::NotStopped {
        what: what.to_string(),
        cause: format!("{why}; {cause}"),
    }
}

/// Read up to `cap` bytes of `path` from offset 0 through a handle of its
/// own, so the read never shares an offset with a process still writing.
/// The flag is true when the file held more than `cap` bytes.
fn read_capped(path: &Path, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    let file = std::fs::File::open(path)?;
    // One byte past the cap tells an overflow from a capture that fits.
    let probe = cap + 1;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    while (bytes.len() as u64) < probe {
        let want = chunk.len().min((probe - bytes.len() as u64) as usize);
        let offset = bytes.len() as u64;
        #[cfg(unix)]
        let got = file.read_at(&mut chunk[..want], offset);
        #[cfg(windows)]
        let got = file.seek_read(&mut chunk[..want], offset);
        match got {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let over = bytes.len() as u64 > cap;
    bytes.truncate(cap as usize);
    Ok((bytes, over))
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::io::Write;

    /// The read stops at the cap and leaves the writer's offset alone, so a
    /// later write lands after, not over, what was already written. Runs on
    /// every platform; unrun on Windows so far.
    #[test]
    fn a_capped_read_never_moves_the_writers_offset() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = file.as_file().try_clone().unwrap();
        writer.write_all(b"0123456789").unwrap();
        assert_eq!(
            read_capped(file.path(), 4).unwrap(),
            (b"0123".to_vec(), true)
        );
        writer.write_all(b"ab").unwrap();
        assert_eq!(
            read_capped(file.path(), 100).unwrap(),
            (b"0123456789ab".to_vec(), false)
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transaction::process_alive;

    fn sh(script: &str) -> Helper {
        let mut helper = Helper::new("/bin/sh");
        helper.args(["-c", script]);
        helper
    }

    fn pid_in(path: &Path) -> u32 {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::fs::read_to_string(path).map_or(true, |s| !s.ends_with('\n')) {
            assert!(
                Instant::now() < deadline,
                "{} never written",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn dies_within(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while process_alive(pid) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn kill(pid: u32) {
        // SAFETY: test cleanup of a process this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }

    /// The child leads a new session, so it has no controlling terminal and
    /// leads its own process group; a login shell run here cannot touch the
    /// caller's terminal. Ids are read while the child still runs.
    #[test]
    fn the_child_leads_its_own_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("child");
        let script = format!("echo $$ > '{}'; sleep 2", file.display());
        let run =
            std::thread::spawn(move || sh(&script).run("session probe", Duration::from_secs(10)));
        wait_for_file(&file);
        let child = pid_in(&file) as libc::pid_t;
        // SAFETY: getsid and getpgid only read ids.
        let (session, group, ours) =
            unsafe { (libc::getsid(child), libc::getpgid(child), libc::getsid(0)) };
        run.join().unwrap().unwrap();
        assert_eq!(session, child, "child leads a new session");
        assert_eq!(group, child, "child leads its process group");
        assert_ne!(session, ours, "child left the caller's session");
    }

    /// Stint 1610: a background job the child started dies when the child
    /// exits, so nothing keeps writing into the capture after the run.
    #[test]
    fn a_background_job_dies_with_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = Instant::now();
        let output = sh(&format!(
            "sleep 600 & echo $! > '{}'; echo done",
            sleeper.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let sleeper = pid_in(&sleeper);
        let died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "done");
        assert!(died, "background job {sleeper} outlived the child");
    }

    /// Stint 1610: a descendant that moved to its own process group inside
    /// the session dies with the child; the session ends, not just the group.
    #[test]
    fn a_descendant_in_another_group_dies_with_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let mover = dir.path().join("mover");
        sh(&format!(
            "perl -e 'setpgrp(0, 0); sleep 600' & echo $! > '{}'; sleep 0.3",
            mover.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let mover = pid_in(&mover);
        let died = dies_within(mover, Duration::from_secs(5));
        kill(mover);
        assert!(
            died,
            "descendant {mover} in its own group outlived the child"
        );
    }

    /// Stint 1610: a descendant in a new process group that keeps forking
    /// children which fork and exit is caught by the convergence loop.
    #[test]
    fn a_forking_descendant_in_a_new_group_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let pids = dir.path().join("pids");
        let script = format!(
            "perl -e 'setpgrp(0, 0); open(my $f, \">>\", \"{}\") or die; $f->autoflush(1); \
             print $f \"$$\\n\"; while (1) {{ my $c = fork(); if (!$c) {{ my $g = fork(); \
             if (!$g) {{ print $f \"$$\\n\"; sleep 600; exit 0 }} exit 0 }} waitpid($c, 0); \
             select(undef, undef, undef, 0.01) }}' & sleep 0.3",
            pids.display()
        );
        let result = sh(&script).run("test child", Duration::from_secs(30));
        let listed: Vec<u32> = std::fs::read_to_string(&pids)
            .unwrap()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        let survivors: Vec<u32> = listed
            .iter()
            .copied()
            .filter(|&pid| !dies_within(pid, Duration::from_secs(5)))
            .collect();
        for &pid in &listed {
            kill(pid);
        }
        assert!(listed.len() > 2, "the forker never forked: {listed:?}");
        assert!(survivors.is_empty(), "escaped: {survivors:?} ({result:?})");
        result.unwrap();
    }

    /// Stint 1610: an inventory that cannot be read ends nothing and is an
    /// error, never an empty session.
    #[test]
    fn an_unreadable_inventory_is_an_error() {
        fn unreadable() -> std::io::Result<Vec<libc::pid_t>> {
            Err(std::io::Error::other("inventory failed"))
        }
        let error = end_session(i32::MAX, unreadable).unwrap_err();
        assert!(error.to_string().contains("inventory failed"), "{error}");
    }

    /// Stint 1610: proc_listallpids returning 0 is a failed enumeration.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_zero_pid_count_is_an_error() {
        assert!(all_pids_with(|_, _| 0).is_err());
        assert!(all_pids_with(|buffer, _| if buffer.is_null() { 3 } else { 0 }).is_err());
        assert!(all_pids_with(|_, _| -1).is_err());
    }

    /// Stint 1610: the limit is checked before an exit is accepted, so an
    /// exit seen after it is a timeout; a wait never runs past the limit.
    #[test]
    fn the_limit_is_checked_before_an_exit_is_accepted() {
        let now = Instant::now();
        let pause = Duration::from_millis(20);
        assert_eq!(next_step(now, Some(now), true, pause), Step::Stop);
        assert_eq!(next_step(now + pause, Some(now), true, pause), Step::Stop);
        assert_eq!(
            next_step(now, Some(now + Duration::from_secs(1)), true, pause),
            Step::Finish
        );
        assert_eq!(
            next_step(now, Some(now + Duration::from_millis(5)), false, pause),
            Step::Wait(Duration::from_millis(5))
        );
        assert_eq!(
            next_step(now, Some(now + Duration::from_secs(1)), false, pause),
            Step::Wait(pause)
        );
    }

    /// Stint 1626: a host-session run has no wall-clock limit: it never
    /// stops for time, only on exit, cancel or overflow.
    #[test]
    fn a_host_session_run_has_no_deadline() {
        let now = Instant::now();
        let pause = Duration::from_millis(20);
        assert_eq!(next_step(now, None, false, pause), Step::Wait(pause));
        assert_eq!(next_step(now, None, true, pause), Step::Finish);
        let exited = sh("sleep 0.3; exit 4")
            .start("test child")
            .unwrap()
            .wait_for(Deadline::HostSession)
            .unwrap();
        assert_eq!(exited.status.code(), Some(4));
    }

    /// With no time at all, even a child that exits at once is a timeout.
    #[test]
    fn a_zero_limit_never_succeeds() {
        let error = sh("exit 0").run("test child", Duration::ZERO).unwrap_err();
        assert!(matches!(error, RunError::TimedOut { .. }), "{error}");
    }

    /// A process that left the session by calling setsid itself, as the host
    /// does under `host start`, survives the child's exit.
    #[test]
    fn a_process_that_detached_survives_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = dir.path().join("daemon");
        sh(&format!(
            "perl -MPOSIX -e 'POSIX::setsid(); sleep 600' & echo $! > '{}'; sleep 0.3",
            daemon.display()
        ))
        .run("test child", Duration::from_secs(30))
        .unwrap();
        let daemon = pid_in(&daemon);
        std::thread::sleep(Duration::from_millis(300));
        let alive = process_alive(daemon);
        kill(daemon);
        assert!(alive, "detached process {daemon} was killed");
    }

    /// Stint 1610: output of exactly the cap is whole; one byte more is an
    /// error, never cut bytes.
    #[test]
    fn output_past_the_cap_is_an_error() {
        let fits = sh(&format!("head -c {CAPTURE_LIMIT} /dev/zero"))
            .run("test child", Duration::from_secs(30))
            .unwrap();
        assert_eq!(fits.stdout.len() as u64, CAPTURE_LIMIT);
        let error = sh(&format!("head -c {} /dev/zero", CAPTURE_LIMIT + 1))
            .run("test child", Duration::from_secs(30))
            .unwrap_err();
        assert!(
            matches!(
                error,
                RunError::OutputTooLarge {
                    stream: "stdout",
                    ..
                }
            ),
            "{error}"
        );
    }

    /// A non-zero exit is `Failed` with its stderr from `run`, and Ok with
    /// the code from `run_any_exit`.
    #[test]
    fn a_failed_exit_is_an_error_unless_the_caller_reads_the_code() {
        let error = sh("echo why >&2; exit 3")
            .run("test child", Duration::from_secs(10))
            .unwrap_err();
        let text = error.to_string();
        assert!(
            matches!(&error, RunError::Failed { status, .. } if status.code() == Some(3)),
            "{text}"
        );
        assert!(text.contains("why"), "{text}");
        let exited = sh("echo why >&2; exit 3")
            .run_any_exit("test child", Duration::from_secs(10))
            .unwrap();
        assert_eq!(exited.status.code(), Some(3));
        assert_eq!(exited.output.stderr, b"why\n");
    }

    /// Stint 1610: git never prompts. Every prompt route is shut.
    #[test]
    fn git_runs_with_every_prompt_route_shut() {
        let helper = git();
        let envs: std::collections::HashMap<_, _> = helper
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(|v| v.to_owned())))
            .collect();
        let env = |key: &str| envs.get(OsStr::new(key)).cloned();
        assert_eq!(env("GIT_TERMINAL_PROMPT"), Some(Some("0".into())));
        assert_eq!(env("GIT_ASKPASS"), Some(Some("".into())));
        assert_eq!(env("SSH_ASKPASS_REQUIRE"), Some(Some("never".into())));
        assert_eq!(env("SSH_ASKPASS"), Some(None));
    }

    /// Stint 1610: a git auth failure says what failed and how to set up
    /// credentials; other failures carry no auth advice.
    #[test]
    fn a_git_auth_failure_says_how_to_set_up_credentials() {
        let failure = |stderr: &str| {
            sh(&format!("echo \"{stderr}\" >&2; exit 128"))
                .run(
                    "git clone https://example.invalid/r.git",
                    Duration::from_secs(10),
                )
                .unwrap_err()
        };
        let auth = git_error(&failure(
            "fatal: could not read Username for https://example.invalid: terminal prompts disabled",
        ));
        assert!(
            auth.starts_with("git clone https://example.invalid/r.git failed"),
            "{auth}"
        );
        assert!(auth.contains("gh auth login"), "{auth}");
        assert!(auth.contains("ssh key"), "{auth}");
        let other = git_error(&failure("error: pathspec v9 did not match"));
        assert!(!other.contains("gh auth"), "{other}");
    }

    /// A child the kill cannot stop does not hold the run: the reap gives up
    /// at its own bound and the error says the child may still be running.
    /// The kill is stubbed to fail, as a real one rarely does.
    #[test]
    fn a_failed_kill_returns_instead_of_waiting_forever() {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 600"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let refuse = |_: &mut Child| Err(std::io::Error::other("refused"));
            let error = stop(
                child,
                "test child",
                Stopped::Cancelled,
                refuse,
                Duration::from_millis(300),
            );
            let _ = tx.send(error);
        });
        let error = rx.recv_timeout(Duration::from_secs(10));
        kill(pid);
        let error = error.expect("stop returned within its reap bound");
        let text = error.to_string();
        assert!(matches!(error, RunError::NotStopped { .. }), "{text}");
        assert!(text.contains("refused"), "{text}");
        assert!(text.contains("may still be running"), "{text}");
    }

    /// Stint 1610: a refused kill stays `NotStopped` even when the child then
    /// exits by itself: its descendants may still be alive.
    #[test]
    fn a_failed_kill_is_not_stopped_even_if_the_child_then_exits() {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 0.2"])
            .spawn()
            .unwrap();
        let refuse = |_: &mut Child| Err(std::io::Error::other("refused"));
        let error = stop(
            child,
            "test child",
            Stopped::Cancelled,
            refuse,
            Duration::from_secs(5),
        );
        let text = error.to_string();
        assert!(matches!(error, RunError::NotStopped { .. }), "{text}");
        assert!(text.contains("refused"), "{text}");
    }

    /// Stint 1626: setting the cancel flag stops the child and its session
    /// well inside the limit, and the run says it was cancelled.
    #[test]
    fn a_cancelled_run_stops_the_child_and_its_session() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let flag = Arc::new(AtomicBool::new(false));
        let script = format!("sleep 600 & echo $! > '{}'; wait", sleeper.display());
        let run = {
            let flag = Arc::clone(&flag);
            std::thread::spawn(move || {
                sh(&script)
                    .cancel_on(flag)
                    .run("test child", Duration::from_secs(600))
            })
        };
        wait_for_file(&sleeper);
        let started = Instant::now();
        flag.store(true, Ordering::SeqCst);
        let error = run.join().unwrap().unwrap_err();
        let sleeper = pid_in(&sleeper);
        let died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(matches!(error, RunError::Cancelled { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5), "{error}");
        assert!(died, "descendant {sleeper} survived the cancel");
    }

    /// Stint 1626: a started helper dropped without a wait is stopped with
    /// its session, never left running unsupervised.
    #[test]
    fn a_started_helper_dropped_unwaited_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = sh(&format!(
            "sleep 600 & echo $! > '{}'; wait",
            sleeper.display()
        ))
        .start("test child")
        .unwrap();
        let child = started.id();
        wait_for_file(&sleeper);
        drop(started);
        let sleeper = pid_in(&sleeper);
        let died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(!process_alive(child), "child {child} survived the drop");
        assert!(died, "descendant {sleeper} survived the drop");
    }

    /// Stint 1626: a writer past its stdout limit is stopped while it runs,
    /// not left to fill the disk until its time limit.
    #[test]
    fn a_runaway_writer_is_stopped_at_its_stdout_limit() {
        let started = Instant::now();
        let error = sh("yes")
            .stdout(OutputPolicy::File { limit: 4096 })
            .run("test child", Duration::from_secs(600))
            .unwrap_err();
        assert!(
            matches!(
                error,
                RunError::OutputTooLarge {
                    stream: "stdout",
                    limit: 4096,
                    ..
                }
            ),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
    }

    /// Stint 1626: input reaches the child's stdin whole and closed, so a
    /// reader to end of file finishes.
    #[test]
    fn input_reaches_stdin_and_ends() {
        let output = sh("cat")
            .input(b"secret value\n".to_vec())
            .run("test child", Duration::from_secs(10))
            .unwrap();
        assert_eq!(output.stdout, b"secret value\n");
    }

    /// Stint 1626: input far larger than a pipe holds reaches a child that
    /// reads it.
    #[test]
    fn input_larger_than_a_pipe_reaches_a_reader() {
        let size = 4 * 1024 * 1024;
        let output = sh("wc -c")
            .input(vec![b'x'; size])
            .run("test child", Duration::from_secs(30))
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            size.to_string()
        );
    }

    /// Stint 1626: a child that never reads its large input still ends at
    /// its limit; the blocked writer does not hold the run.
    #[test]
    fn a_child_that_never_reads_its_input_still_times_out() {
        let started = Instant::now();
        let error = sh("sleep 600")
            .input(vec![b'x'; 16 * 1024 * 1024])
            .run("test child", Duration::from_millis(500))
            .unwrap_err();
        assert!(matches!(error, RunError::TimedOut { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(8), "{error}");
    }

    /// Stint 1626: discarded output is never captured, so its size cannot
    /// fail the run.
    #[test]
    fn discarded_output_cannot_fail_the_run() {
        let output = sh(&format!(
            "head -c {} /dev/zero >&2; head -c {} /dev/zero",
            CAPTURE_LIMIT * 2,
            CAPTURE_LIMIT * 2
        ))
        .stdout(OutputPolicy::Discard)
        .stderr(OutputPolicy::Discard)
        .run("test child", Duration::from_secs(30))
        .unwrap();
        assert_eq!(output, Output::default());
    }

    /// Stint 1626 (Codex round 2): an input thread whose reader never reads
    /// exits once its run ends, so it pins neither the pipe nor the secret
    /// bytes it owns (dropping them zeroes them).
    #[test]
    fn an_input_thread_with_a_reader_that_never_reads_exits_when_the_run_ends() {
        let (reader, writer) = std::io::pipe().unwrap();
        let delivery =
            Delivery::spawn(writer, Zeroizing::new(vec![b'x'; 16 * 1024 * 1024])).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !delivery.done.load(Ordering::SeqCst),
            "the pipe should be full and the thread waiting"
        );
        let done = Arc::clone(&delivery.done);
        drop(delivery);
        let exited = wait_flag(&done, Duration::from_secs(2));
        drop(reader);
        assert!(exited, "the input thread outlived its run");
    }

    /// Stint 1626 (Codex round 2): a descendant that leaves the session while
    /// holding stdin, never reading it, does not hold the run: it returns an
    /// error within its drain bound.
    #[test]
    fn a_detached_descendant_holding_stdin_does_not_hold_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let holder = dir.path().join("holder");
        let started = Instant::now();
        let error = sh(&format!(
            // A background job's stdin is /dev/null unless redirected
            // explicitly, so hand it the real stdin through fd 3.
            "exec 3<&0; perl -MPOSIX -e 'POSIX::setsid(); sleep 600' <&3 & echo $! > '{}'; sleep 0.3",
            holder.display()
        ))
        .input(vec![b'x'; 16 * 1024 * 1024])
        .run("test child", Duration::from_secs(30))
        .unwrap_err();
        let elapsed = started.elapsed();
        kill(pid_in(&holder));
        assert!(
            matches!(
                &error,
                RunError::Io {
                    action: "write stdin",
                    ..
                }
            ),
            "{error}"
        );
        assert!(elapsed < Duration::from_secs(10), "held for {elapsed:?}");
    }

    /// Stint 1626: a memory capture is read whole without a temp file, and
    /// past its limit stops the child as `OutputTooLarge` on that stream.
    #[test]
    fn a_memory_capture_is_whole_and_bounded() {
        let output = sh("printf secret-value")
            .stdout(OutputPolicy::Memory { limit: 64 })
            .run("test child", Duration::from_secs(10))
            .unwrap();
        assert_eq!(output.stdout, b"secret-value");
        let started = Instant::now();
        let error = sh("yes")
            .stdout(OutputPolicy::Memory { limit: 64 })
            .run("test child", Duration::from_secs(600))
            .unwrap_err();
        assert!(
            matches!(
                error,
                RunError::OutputTooLarge {
                    stream: "stdout",
                    limit: 64,
                    ..
                }
            ),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
    }

    /// Stint 1626: a memory capture never creates a temp file: both streams
    /// are pipes drained into memory.
    #[test]
    fn a_memory_capture_never_touches_disk() {
        let started = sh("sleep 0.3; printf secret-value")
            .stdout(OutputPolicy::Memory { limit: 64 })
            .stderr(OutputPolicy::Memory { limit: 64 })
            .start("test child")
            .unwrap();
        let Sink::Pipe { .. } = &started.stdout else {
            panic!("stdout is not a pipe");
        };
        let Sink::Pipe { .. } = &started.stderr else {
            panic!("stderr is not a pipe");
        };
        assert_eq!(
            started.wait(Duration::from_secs(10)).unwrap().output.stdout,
            b"secret-value"
        );
    }

    /// Stint 1626: a tail policy drains a stream of any size, keeps its last
    /// bytes and never fails the run.
    #[test]
    fn a_tail_keeps_the_end_and_never_fails() {
        let output = sh(&format!(
            "head -c {} /dev/zero >&2; printf the-end >&2; echo ok",
            CAPTURE_LIMIT * 3
        ))
        .stderr(OutputPolicy::Tail { keep: 16 })
        .run("test child", Duration::from_secs(30))
        .unwrap();
        assert_eq!(output.stderr.len(), 16);
        assert!(output.stderr.ends_with(b"the-end"), "{:?}", output.stderr);
        assert_eq!(output.stdout, b"ok\n");
    }

    /// Stint 1626: when checking for the child's exit fails, the session is
    /// still stopped and reaped before the error returns.
    #[test]
    fn a_failed_exit_check_still_stops_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = dir.path().join("sleeper");
        let started = sh(&format!(
            "sleep 600 & echo $! > '{}'; wait",
            sleeper.display()
        ))
        .start("test child")
        .unwrap();
        let child = started.id();
        wait_for_file(&sleeper);
        let failing = |_: &mut Child| Err(std::io::Error::other("injected wait failure"));
        let error = started
            .wait_with(Deadline::After(Duration::from_secs(600)), failing)
            .unwrap_err();
        let sleeper = pid_in(&sleeper);
        let died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(
            error.to_string().contains("injected wait failure"),
            "{error}"
        );
        assert!(
            !process_alive(child),
            "child {child} survived the failed wait"
        );
        assert!(died, "descendant {sleeper} survived the failed wait");
    }

    /// `arg0` is what the program sees as its name.
    #[test]
    fn arg0_names_the_program() {
        let output = Helper::new("/bin/sh")
            .arg0("shown-name")
            .args(["-c", "echo $0"])
            .run("test child", Duration::from_secs(10))
            .unwrap();
        assert_eq!(output.stdout, b"shown-name\n");
    }

    /// A hung child and its descendants are killed at the limit and reaped,
    /// and what it wrote before the hang is in the error.
    #[test]
    fn a_hung_child_and_its_descendants_are_killed_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (child, sleeper) = (dir.path().join("child"), dir.path().join("sleeper"));
        let started = Instant::now();
        let error = sh(&format!(
            "echo $$ > '{}'; sleep 600 & echo $! > '{}'; echo partial; wait",
            child.display(),
            sleeper.display()
        ))
        .run("test child", Duration::from_millis(500))
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        let RunError::TimedOut { stdout, .. } = &error else {
            panic!("expected TimedOut: {error}");
        };
        assert_eq!(stdout, b"partial\n");
        let (child, sleeper) = (pid_in(&child), pid_in(&sleeper));
        let sleeper_died = dies_within(sleeper, Duration::from_secs(5));
        kill(sleeper);
        assert!(!process_alive(child), "child {child} survived");
        assert!(sleeper_died, "descendant {sleeper} survived");
    }
}
