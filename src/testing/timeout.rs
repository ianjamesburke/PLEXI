use std::time::Duration;

/// Returns a correctness timeout scaled for the machine's current one-minute
/// load average. Wall-clock budgets prove that an asynchronous operation
/// eventually completes; they are not performance claims, so a busy fleet
/// must not turn them into false failures. The multiplier is capped to keep a
/// genuine hang bounded even when the host is severely overloaded.
#[cfg(unix)]
pub(crate) fn load_aware_timeout(base: Duration) -> Duration {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get() as f64)
        .unwrap_or(1.0);
    let mut load = [0.0; 3];
    let samples = unsafe { libc::getloadavg(load.as_mut_ptr(), load.len() as i32) };
    let multiplier = if samples > 0 {
        load_multiplier(load[0], cores)
    } else {
        1
    };
    base.saturating_mul(multiplier)
}

// Windows does not expose a Unix load average. Keep the declared correctness
// budget; do not invoke an unavailable libc entry point in native tests.
#[cfg(windows)]
pub(crate) fn load_aware_timeout(base: Duration) -> Duration {
    base
}

#[cfg(any(unix, test))]
fn load_multiplier(load: f64, cores: f64) -> u32 {
    const MAX_MULTIPLIER: u32 = 4;
    (load / cores).ceil().clamp(1.0, MAX_MULTIPLIER as f64) as u32
}

#[cfg(test)]
mod load_aware_timeout_tests {
    use super::*;

    #[test]
    fn correctness_budget_scales_with_load_and_stops_at_the_cap() {
        let base = Duration::from_secs(10);
        assert_eq!(base.saturating_mul(load_multiplier(0.5, 4.0)), base);
        assert_eq!(
            base.saturating_mul(load_multiplier(7.0, 4.0)),
            Duration::from_secs(20)
        );
        assert_eq!(
            base.saturating_mul(load_multiplier(100.0, 4.0)),
            Duration::from_secs(40)
        );
    }
}
