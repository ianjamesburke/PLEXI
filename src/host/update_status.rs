//! Update status: the host's view of the background update check.
//!
//! Pure state, no egui. The updater thread (`cli::updater::spawn_update_check`)
//! reports [`UpdateEvent`]s; the host applies them here; the palette, toolbar
//! and changelog view read [`UpdateStatus::line`] and [`UpdateStatus::ready`].
//! A downloaded build stays ready until the host restarts into it, even if a
//! later check fails; a later UpToDate means it was discarded and clears it.

/// Why a check was started. A manual check skips the update cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckMode {
    Scheduled,
    Manual,
}

impl CheckMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CheckMode::Scheduled => "scheduled",
            CheckMode::Manual => "manual",
        }
    }
}

/// What the updater thread reports. Every check ends with exactly one of
/// `Ready`, `UpToDate`, `Failed` or `Unavailable`; `Downloading` may precede
/// `Ready` or `Failed`. Versions carry no leading `v`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateEvent {
    Downloading { version: String },
    Ready { version: String },
    UpToDate { checked_at: u64 },
    Failed { reason: String },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InFlight {
    Checking,
    Downloading(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    UpToDate { checked_at: u64 },
    Failed { reason: String },
    Unavailable { reason: String },
}

/// The one line the changelog view shows. Ordered by what the user can act on:
/// work in progress first, then a ready build, then the last outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateLine {
    NotChecked,
    Checking,
    Downloading { version: String },
    Ready { version: String },
    UpToDate { checked_at: u64 },
    Failed { reason: String },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UpdateStatus {
    in_flight: Option<InFlight>,
    ready: Option<String>,
    last: Option<Outcome>,
}

impl UpdateStatus {
    /// Mark a check as started. Returns `false`, changing nothing, while a
    /// check is already running: one check at a time.
    pub(crate) fn begin_check(&mut self) -> bool {
        if self.in_flight.is_some() {
            return false;
        }
        self.in_flight = Some(InFlight::Checking);
        true
    }

    pub(crate) fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// The downloaded build waiting for a restart, without a leading `v`.
    pub(crate) fn ready(&self) -> Option<&str> {
        self.ready.as_deref()
    }

    pub(crate) fn apply(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::Downloading { version } => {
                self.in_flight = Some(InFlight::Downloading(version));
            }
            UpdateEvent::Ready { version } => {
                self.in_flight = None;
                self.ready = Some(version);
            }
            UpdateEvent::UpToDate { checked_at } => {
                // The updater reports UpToDate only when the installed build is
                // the running one, so a build offered earlier was discarded.
                if let Some(version) = self.ready.take() {
                    log::info!("update status: v{version} is no longer waiting for a restart");
                }
                self.in_flight = None;
                self.last = Some(Outcome::UpToDate { checked_at });
            }
            UpdateEvent::Failed { reason } => {
                self.in_flight = None;
                self.last = Some(Outcome::Failed { reason });
            }
            UpdateEvent::Unavailable { reason } => {
                self.in_flight = None;
                self.last = Some(Outcome::Unavailable { reason });
            }
        }
    }

    /// The updater thread went away mid-check without reporting an outcome.
    /// No-op when nothing was in flight.
    pub(crate) fn ended_without_result(&mut self) {
        if self.in_flight.is_some() {
            self.apply(UpdateEvent::Failed {
                reason: "the update check stopped without a result".into(),
            });
        }
    }

    pub(crate) fn line(&self) -> UpdateLine {
        match (&self.in_flight, &self.ready, &self.last) {
            (Some(InFlight::Downloading(version)), _, _) => UpdateLine::Downloading {
                version: version.clone(),
            },
            (Some(InFlight::Checking), _, _) => UpdateLine::Checking,
            (None, Some(version), _) => UpdateLine::Ready {
                version: version.clone(),
            },
            (None, None, Some(Outcome::UpToDate { checked_at })) => UpdateLine::UpToDate {
                checked_at: *checked_at,
            },
            (None, None, Some(Outcome::Failed { reason })) => UpdateLine::Failed {
                reason: reason.clone(),
            },
            (None, None, Some(Outcome::Unavailable { reason })) => UpdateLine::Unavailable {
                reason: reason.clone(),
            },
            (None, None, None) => UpdateLine::NotChecked,
        }
    }
}

/// Status text after the running version, e.g. "Up to date · checked 5 min ago".
pub(crate) fn status_text(line: &UpdateLine, now: u64) -> String {
    match line {
        UpdateLine::NotChecked => "Not checked yet".into(),
        UpdateLine::Checking => "Checking for updates\u{2026}".into(),
        UpdateLine::Downloading { version } => format!("Downloading v{version}\u{2026}"),
        UpdateLine::Ready { version } => format!("v{version} is ready"),
        UpdateLine::UpToDate { checked_at } => {
            format!("Up to date · checked {}", checked_ago(now, *checked_at))
        }
        UpdateLine::Failed { reason } => format!("Update check failed: {reason}"),
        UpdateLine::Unavailable { reason } => reason.clone(),
    }
}

fn checked_ago(now: u64, checked_at: u64) -> String {
    let secs = now.saturating_sub(checked_at);
    match secs {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        _ => {
            let days = secs / 86_400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_status_starts_not_checked() {
        let status = UpdateStatus::default();
        assert_eq!(status.line(), UpdateLine::NotChecked);
        assert!(!status.in_flight());
        assert_eq!(status.ready(), None);
    }

    #[test]
    fn update_status_check_download_ready() {
        let mut status = UpdateStatus::default();
        assert!(status.begin_check());
        assert_eq!(status.line(), UpdateLine::Checking);
        status.apply(UpdateEvent::Downloading {
            version: "0.5.1".into(),
        });
        assert_eq!(
            status.line(),
            UpdateLine::Downloading {
                version: "0.5.1".into()
            }
        );
        assert_eq!(status.ready(), None);
        status.apply(UpdateEvent::Ready {
            version: "0.5.1".into(),
        });
        assert!(!status.in_flight());
        assert_eq!(status.ready(), Some("0.5.1"));
        assert_eq!(
            status.line(),
            UpdateLine::Ready {
                version: "0.5.1".into()
            }
        );
    }

    #[test]
    fn update_status_refuses_second_check_while_one_runs() {
        let mut status = UpdateStatus::default();
        assert!(status.begin_check());
        assert!(!status.begin_check());
        status.apply(UpdateEvent::Downloading {
            version: "0.5.1".into(),
        });
        assert!(!status.begin_check());
        status.apply(UpdateEvent::UpToDate { checked_at: 10 });
        assert!(status.begin_check());
    }

    #[test]
    fn update_status_failure_after_ready_keeps_the_ready_build() {
        let mut status = UpdateStatus::default();
        status.begin_check();
        status.apply(UpdateEvent::Ready {
            version: "0.5.1".into(),
        });
        status.begin_check();
        status.apply(UpdateEvent::Failed {
            reason: "network down".into(),
        });
        assert_eq!(status.ready(), Some("0.5.1"));
        assert_eq!(
            status.line(),
            UpdateLine::Ready {
                version: "0.5.1".into()
            }
        );
    }

    #[test]
    fn update_status_up_to_date_after_ready_clears_a_discarded_build() {
        let mut status = UpdateStatus::default();
        status.begin_check();
        status.apply(UpdateEvent::Ready {
            version: "0.5.0-beta.1".into(),
        });
        status.begin_check();
        status.apply(UpdateEvent::UpToDate { checked_at: 7 });
        assert_eq!(status.ready(), None);
        assert_eq!(status.line(), UpdateLine::UpToDate { checked_at: 7 });
    }

    #[test]
    fn update_status_reports_up_to_date_failed_and_unavailable() {
        let mut status = UpdateStatus::default();
        status.begin_check();
        status.apply(UpdateEvent::UpToDate { checked_at: 100 });
        assert_eq!(status.line(), UpdateLine::UpToDate { checked_at: 100 });
        status.begin_check();
        status.apply(UpdateEvent::Failed {
            reason: "HTTP 503".into(),
        });
        assert_eq!(
            status.line(),
            UpdateLine::Failed {
                reason: "HTTP 503".into()
            }
        );
        status.begin_check();
        status.apply(UpdateEvent::Unavailable {
            reason: "pr-7 builds do not check for updates".into(),
        });
        assert_eq!(
            status.line(),
            UpdateLine::Unavailable {
                reason: "pr-7 builds do not check for updates".into()
            }
        );
    }

    #[test]
    fn update_status_thread_exit_without_result_is_a_failure() {
        let mut status = UpdateStatus::default();
        status.ended_without_result();
        assert_eq!(status.line(), UpdateLine::NotChecked);
        status.begin_check();
        status.ended_without_result();
        assert!(!status.in_flight());
        assert!(matches!(status.line(), UpdateLine::Failed { .. }));
    }

    #[test]
    fn update_status_text_names_each_state() {
        assert_eq!(
            status_text(&UpdateLine::Checking, 0),
            "Checking for updates…"
        );
        assert_eq!(
            status_text(
                &UpdateLine::Downloading {
                    version: "0.5.1".into()
                },
                0
            ),
            "Downloading v0.5.1…"
        );
        assert_eq!(
            status_text(&UpdateLine::UpToDate { checked_at: 1_000 }, 1_030),
            "Up to date · checked just now"
        );
        assert_eq!(
            status_text(&UpdateLine::UpToDate { checked_at: 1_000 }, 1_000 + 5 * 60),
            "Up to date · checked 5 min ago"
        );
        assert_eq!(
            status_text(&UpdateLine::UpToDate { checked_at: 0 }, 2 * 3_600),
            "Up to date · checked 2 h ago"
        );
        assert_eq!(
            status_text(&UpdateLine::UpToDate { checked_at: 0 }, 86_400),
            "Up to date · checked 1 day ago"
        );
        assert_eq!(
            status_text(
                &UpdateLine::Failed {
                    reason: "HTTP 503".into()
                },
                0
            ),
            "Update check failed: HTTP 503"
        );
    }
}
