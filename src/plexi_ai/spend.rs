//! Spend admission for brokered AI calls (stint 1691).
//!
//! Every paid call takes a [`SpendReservation`] before dispatch. Admission
//! reads today's ledger spend and the holds of calls still running under one
//! lock, so concurrent calls from any pane or runtime cannot all pass against
//! the same remaining budget. A call that cannot be recorded blocks further
//! spend until its row reaches the ledger.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::config::AiConfig;
use crate::plexi_ai::ledger::{self, LedgerRow};

/// USD held for each admitted call until its ledger row is written. Plexi
/// has no per-model price table or output-token ceiling, so this is a fixed
/// estimate rather than a true worst case: a single call that costs more than
/// it settles above its hold.
pub const PER_CALL_RESERVE_USD: f64 = 0.10;

/// Paid calls that may run at once across every pane and runtime.
pub const MAX_ACTIVE_CALLS: usize = 16;

/// Why admission refused a call. [`Self::code`] is the stable prefix the
/// broker returns to its caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpendDenial {
    /// The ledger cannot be read, so today's spend is unknown.
    LedgerUnavailable(String),
    /// An earlier paid call is still unrecorded and the ledger still refuses it.
    AccountingFailed(String),
    /// A daily cap would be passed by this call's hold.
    Exceeded(String),
    /// [`MAX_ACTIVE_CALLS`] calls are already running.
    Busy(String),
}

impl SpendDenial {
    pub fn code(&self) -> &'static str {
        match self {
            Self::LedgerUnavailable(_) => "ledger_unavailable",
            Self::AccountingFailed(_) => "accounting_failed",
            Self::Exceeded(_) => "budget_exceeded",
            Self::Busy(_) => "ai_busy",
        }
    }
}

impl std::fmt::Display for SpendDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LedgerUnavailable(m)
            | Self::AccountingFailed(m)
            | Self::Exceeded(m)
            | Self::Busy(m) => f.write_str(m),
        }
    }
}

/// Admission state for one ledger file. Keyed by path so isolated test
/// profiles never share holds.
#[derive(Default)]
struct LedgerAdmission {
    next_id: u64,
    holds: HashMap<u64, Hold>,
    /// Rows for calls that were paid but could not be appended.
    unrecorded: Vec<LedgerRow>,
}

struct Hold {
    plugin_id: String,
    usd: f64,
}

fn state() -> MutexGuard<'static, HashMap<PathBuf, LedgerAdmission>> {
    static STATE: OnceLock<Mutex<HashMap<PathBuf, LedgerAdmission>>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A held share of today's budget. Dropping it releases the hold, which is
/// the failure path: a call that never reached the provider costs nothing.
#[must_use = "dropping a reservation releases its hold"]
pub struct SpendReservation {
    ledger: PathBuf,
    id: u64,
}

/// Admit one call from `plugin_id` under `config`'s caps, or refuse it.
pub fn reserve(plugin_id: &str, config: &AiConfig) -> Result<SpendReservation, SpendDenial> {
    if let Some(error) = ledger::retired_ledger_error() {
        return Err(SpendDenial::LedgerUnavailable(error));
    }
    let path = ledger::ledger_path();
    let mut all = state();
    let admission = all.entry(path.clone()).or_default();

    if !admission.unrecorded.is_empty() {
        flush_unrecorded(admission)?;
    }
    if admission.holds.len() >= MAX_ACTIVE_CALLS {
        return Err(SpendDenial::Busy(format!(
            "{MAX_ACTIVE_CALLS} AI calls are already running; retry when one finishes"
        )));
    }
    let spend = ledger::today_spend().map_err(SpendDenial::LedgerUnavailable)?;
    let held_global: f64 = admission.holds.values().map(|h| h.usd).sum();
    let held_plugin: f64 = admission
        .holds
        .values()
        .filter(|h| h.plugin_id == plugin_id)
        .map(|h| h.usd)
        .sum();

    let global_cap = config.effective_global_daily_usd();
    let global_committed = spend.global_usd + held_global;
    if global_committed + PER_CALL_RESERVE_USD > global_cap {
        return Err(SpendDenial::Exceeded(format!(
            "global daily AI budget exceeded (${:.2} spent or held of ${:.2})",
            global_committed, global_cap
        )));
    }
    let plugin_cap = config.effective_per_plugin_daily_usd();
    let plugin_committed = spend.per_plugin.get(plugin_id).copied().unwrap_or(0.0) + held_plugin;
    if plugin_committed + PER_CALL_RESERVE_USD > plugin_cap {
        return Err(SpendDenial::Exceeded(format!(
            "per-plugin daily AI budget exceeded for '{plugin_id}' (${:.2} spent or held of ${:.2})",
            plugin_committed, plugin_cap
        )));
    }

    admission.next_id += 1;
    let id = admission.next_id;
    admission.holds.insert(
        id,
        Hold {
            plugin_id: plugin_id.to_string(),
            usd: PER_CALL_RESERVE_USD,
        },
    );
    log::info!(
        "ai_spend[{plugin_id}]: reserved ${PER_CALL_RESERVE_USD:.2} active={} global_committed=${global_committed:.4} plugin_committed=${plugin_committed:.4}",
        admission.holds.len()
    );
    Ok(SpendReservation { ledger: path, id })
}

/// Append rows that an earlier call could not write. Any failure keeps them
/// queued and refuses admission.
fn flush_unrecorded(admission: &mut LedgerAdmission) -> Result<(), SpendDenial> {
    while let Some(row) = admission.unrecorded.first() {
        if let Err(error) = ledger::append_result(row) {
            return Err(SpendDenial::AccountingFailed(format!(
                "{} paid AI call(s) are not in the ledger and it still refuses writes: {error}",
                admission.unrecorded.len()
            )));
        }
        admission.unrecorded.remove(0);
        log::info!("ai_spend: recorded a previously unrecorded ledger row");
    }
    Ok(())
}

impl SpendReservation {
    /// Write the call's ledger row. The hold stays until the reservation is
    /// dropped, so a caller still filling the row's cost keeps it held. On
    /// failure the row is queued and every later admission is refused until
    /// it can be written.
    pub fn record(&self, row: &LedgerRow) -> Result<(), String> {
        let mut all = state();
        match ledger::append_result(row) {
            Ok(()) => Ok(()),
            Err(error) => {
                all.entry(self.ledger.clone())
                    .or_default()
                    .unrecorded
                    .push(row.clone());
                Err(error)
            }
        }
    }
}

impl Drop for SpendReservation {
    fn drop(&mut self) {
        let mut all = state();
        if let Some(admission) = all.get_mut(&self.ledger) {
            admission.holds.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(per_plugin: f64, global: f64) -> AiConfig {
        AiConfig {
            per_plugin_daily_usd: Some(per_plugin),
            global_daily_usd: Some(global),
            ..Default::default()
        }
    }

    #[test]
    fn a_zero_global_cap_refuses_with_budget_exceeded() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let denial = reserve("app1", &caps(1.0, 0.0)).err().expect("refused");
        assert_eq!(denial.code(), "budget_exceeded");
        assert!(denial.to_string().contains("global daily"), "{denial}");
    }

    #[test]
    fn a_zero_plugin_cap_refuses_naming_the_plugin() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let denial = reserve("app1", &caps(0.0, 100.0)).err().expect("refused");
        assert_eq!(denial.code(), "budget_exceeded");
        assert!(denial.to_string().contains("'app1'"), "{denial}");
    }

    #[test]
    fn held_reservations_count_against_the_cap_until_dropped() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let config = caps(PER_CALL_RESERVE_USD * 1.5, 100.0);
        let first = reserve("app1", &config).expect("first call fits");
        assert_eq!(
            reserve("app1", &config).err().map(|d| d.code()),
            Some("budget_exceeded")
        );
        drop(first);
        let _held = reserve("app1", &config).expect("a released hold frees the budget");
    }

    #[test]
    fn active_calls_are_bounded() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let config = caps(1000.0, 1000.0);
        let held: Vec<_> = (0..MAX_ACTIVE_CALLS)
            .map(|i| reserve(&format!("app{i}"), &config).expect("under the bound"))
            .collect();
        assert_eq!(
            reserve("one-more", &config).err().map(|d| d.code()),
            Some("ai_busy")
        );
        drop(held);
        let _held = reserve("one-more", &config).expect("bound frees as calls finish");
    }
}
