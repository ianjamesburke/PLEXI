//! The one `log` logger for the unit-test binary (stint 1507).
//!
//! `log` allows a single logger per process, so every test that needs log
//! records goes through this module instead of installing its own.
//!
//! - Warn and error records are printed with `println!`. libtest captures a
//!   test thread's stdout and shows it only when that test fails, so a failing
//!   test carries the reason the host logged (for example a terminal whose PTY
//!   spawn failed) without a test-only hook in production code.
//! - [`capture`] collects the info-and-above records logged on the calling
//!   thread, for tests that assert on a trace.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

thread_local! {
    static CAPTURED: RefCell<Option<Vec<(log::Level, String)>>> = const { RefCell::new(None) };
}

struct TestLogger;

impl log::Log for TestLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = record.args().to_string();
        if record.level() <= log::Level::Warn {
            println!("[{} {}] {line}", record.level(), record.target());
        }
        let _ = CAPTURED.try_with(|slot| {
            if let Some(records) = slot.borrow_mut().as_mut() {
                records.push((record.level(), line));
            }
        });
    }

    fn flush(&self) {}
}

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the test logger once per process. A test that deliberately runs
/// the production logger (`platform::logging::init`, always in its own child
/// process) keeps it: the harness then leaves logging alone.
pub(crate) fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if log::set_boxed_logger(Box::new(TestLogger)).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
            INSTALLED.store(true, Ordering::SeqCst);
        }
    });
}

/// Records logged on this thread while the guard lives.
pub(crate) struct LogCapture(());

/// Start collecting this thread's info-and-above records. Installs the logger.
pub(crate) fn capture() -> LogCapture {
    install();
    assert!(
        INSTALLED.load(Ordering::SeqCst),
        "another logger was installed before testing::test_log, so nothing can be captured; \
         route the test's logging through testing::test_log"
    );
    CAPTURED.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
    LogCapture(())
}

impl LogCapture {
    /// The records collected so far, oldest first.
    pub(crate) fn records(&self) -> Vec<(log::Level, String)> {
        CAPTURED.with(|slot| slot.borrow().clone().unwrap_or_default())
    }

    /// Just the message text of the records collected so far.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.records().into_iter().map(|(_, line)| line).collect()
    }
}

impl Drop for LogCapture {
    fn drop(&mut self) {
        let _ = CAPTURED.try_with(|slot| *slot.borrow_mut() = None);
    }
}
