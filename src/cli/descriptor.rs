use crate::plexi_descriptor::{self, PlexiDescriptor};
use std::process::Command;

/// Indirection so the probe path can be tested without spawning real
/// processes. The `&[&str] -> Output` shape is the smallest contract that
/// covers "what command was run with what args".
pub trait DescriptorRunner {
    fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput>;
}

pub struct RunOutput {
    pub status_success: bool,
    pub stdout: String,
}

pub struct RealRunner;

impl DescriptorRunner for RealRunner {
    fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput> {
        let out = Command::new(command).args(args).output()?;
        Ok(RunOutput {
            status_success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        })
    }
}

/// Knobs governing the Tier-2 registry and Tier-3 crawl fallbacks. The
/// default behavior (Tier 1 first, Tier 2, then Tier 3) matches the issue
/// #321/#360 substrate; `--no-registry` disables Tier 2 and `--no-crawl`
/// disables Tier 3.
pub struct ProbeOptions {
    pub use_registry: bool,
    pub use_crawl: bool,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            use_registry: true,
            use_crawl: true,
        }
    }
}

/// Run `<command> <args...> --plexi`, parse + summarize. On failure (spawn
/// error, non-zero exit, or unparseable JSON), optionally fall through to
/// the Tier-2 registry lookup (`cli_registry::lookup`). Returns the
/// process exit code suitable for `std::process::exit`.
///
/// The CLI surface in `main.rs` calls `probe_with_options` directly so
/// it can plumb `--no-registry`; this thin wrapper exists for tests and
/// for any future caller that wants the default behavior.
#[cfg(test)]
pub fn probe<R: DescriptorRunner>(runner: &R, command: &str, args: &[&str]) -> i32 {
    probe_with_options(runner, command, args, &ProbeOptions::default())
}

pub fn probe_with_options<R: DescriptorRunner>(
    runner: &R,
    command: &str,
    args: &[&str],
    options: &ProbeOptions,
) -> i32 {
    let mut full_args: Vec<&str> = args.to_vec();
    full_args.push("--plexi");

    // Tier 1 — ask the CLI itself.
    let tier1: Option<PlexiDescriptor> = match runner.run(command, &full_args) {
        Ok(o) if o.status_success => match plexi_descriptor::parse(&o.stdout) {
            Ok(d) => Some(d),
            Err(_) => None, // Fall through to Tier 2 — bad/empty stdout.
        },
        Ok(_) => None, // Non-zero exit — `--plexi` not implemented.
        Err(_) => None, // Spawn failed (e.g. command not on PATH).
    };

    if let Some(descriptor) = tier1 {
        print_summary(&descriptor, SummarySource::Native);
        return 0;
    }

    // Tier 2 — registry. Only consulted when caller passes args=[],
    // because registry descriptors describe the bare CLI, not arbitrary
    // subcommand invocations.
    if options.use_registry && args.is_empty() {
        match crate::cli::registry::lookup(command, None) {
            Ok(descriptor) => {
                print_summary(&descriptor, SummarySource::Registry);
                return 0;
            }
            Err(crate::cli::registry::RegistryError::NotFound { .. }) => {
                // Fall through to the no-descriptor message below.
            }
            Err(e) => {
                eprintln!("error: registry lookup for `{command}` failed:\n  {e}");
                return 1;
            }
        }
    }

    // Tier 3 — --help crawl.
    if options.use_crawl && args.is_empty() {
        match crate::cli::crawl::crawl(command) {
            Ok(result) => {
                print_summary(
                    &result.descriptor,
                    SummarySource::Crawled {
                        from_cache: result.from_cache,
                    },
                );
                return 0;
            }
            Err(e) => {
                log::warn!("cli_crawl: Tier 3 failed for `{command}`: {e}");
            }
        }
    }

    eprintln!("error: no descriptor available for `{command}` — --plexi, registry, and --help crawl all failed.");
    1
}

/// Where the descriptor printed in the summary came from. Used to surface
/// a `(via registry)` / `(inferred from --help)` indicator.
pub enum SummarySource {
    Native,
    Registry,
    Crawled { from_cache: bool },
}

fn print_summary(d: &PlexiDescriptor, source: SummarySource) {
    let icon = d.icon.as_deref().unwrap_or("");
    let via = match source {
        SummarySource::Native => "",
        SummarySource::Registry => "  (via registry)",
        SummarySource::Crawled { from_cache: true } => "  (inferred from --help, cached)",
        SummarySource::Crawled { from_cache: false } => {
            "  (inferred from --help, may be incomplete)"
        }
    };
    println!(
        "{}{}{} v{}  (descriptor {}){}",
        icon,
        if icon.is_empty() { "" } else { " " },
        d.name,
        d.version,
        d.plexi_version,
        via,
    );
    if let Some(desc) = &d.description {
        println!("  {desc}");
    }
    println!("commands: {}", d.commands.len());
    for cmd in d.commands.iter().take(3) {
        let hint = cmd
            .ui_hint
            .map(|h| format!(" [{h:?}]").to_lowercase())
            .unwrap_or_default();
        let extra = if cmd.commands.is_empty() {
            String::new()
        } else {
            format!(" (+{} subcommands)", cmd.commands.len())
        };
        let desc = cmd
            .description
            .as_deref()
            .map(|s| format!(" — {s}"))
            .unwrap_or_default();
        println!("  - {}{hint}{extra}{desc}", cmd.name);
    }
    if d.commands.len() > 3 {
        println!("  ... and {} more", d.commands.len() - 3);
    }
    if let Some(ls) = &d.live_state {
        println!(
            "live_state: {:?} {} (poll {} ms, {:?})",
            ls.source, ls.path, ls.poll_ms, ls.format
        );
    }
    if let Some(app_cmd) = &d.plexi_app {
        println!("plexi_app: {app_cmd}");
        if !d.capabilities.is_empty() {
            println!("  capabilities: {}", d.capabilities.join(", "));
        }
    }
}

#[cfg(test)]
pub struct MockRunner {
    pub stdout: String,
    pub success: bool,
    /// Last (command, args) the probe handed to the runner. Lets tests
    /// assert that `--plexi` was appended in the right position.
    pub captured: std::cell::RefCell<Option<(String, Vec<String>)>>,
}

#[cfg(test)]
impl DescriptorRunner for MockRunner {
    fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput> {
        *self.captured.borrow_mut() =
            Some((command.to_string(), args.iter().map(|s| s.to_string()).collect()));
        Ok(RunOutput {
            status_success: self.success,
            stdout: self.stdout.clone(),
        })
    }
}
#[cfg(test)]
mod descriptor_tests {
    use super::*;
    use std::cell::RefCell;

    fn ok_descriptor_runner() -> MockRunner {
        MockRunner {
            stdout: r#"{
                "plexi_version": "0.1",
                "name": "fake",
                "version": "0.0.1",
                "commands": []
            }"#
            .into(),
            success: true,
            captured: RefCell::new(None),
        }
    }

    fn no_plexi_runner() -> MockRunner {
        // Simulates a CLI that exists on PATH but doesn't implement --plexi
        // (non-zero exit code). This is the common case for the registry
        // fallback path.
        MockRunner {
            stdout: String::new(),
            success: false,
            captured: RefCell::new(None),
        }
    }

    #[test]
    fn probe_invokes_command_with_plexi_flag() {
        let mock = ok_descriptor_runner();
        let code = probe(&mock, "fake-cli", &[]);
        // Tier 1 succeeds; result is the parsed descriptor.
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (cmd, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(cmd, "fake-cli");
        assert_eq!(args.last().map(|s| s.as_str()), Some("--plexi"));
    }

    #[test]
    fn probe_appends_plexi_after_user_args() {
        let mock = ok_descriptor_runner();
        let code = probe(&mock, "fake-cli", &["sub", "--verbose"]);
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (_, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(args.as_slice(), &["sub", "--verbose", "--plexi"]);
    }

    #[test]
    fn probe_falls_back_to_registry_when_native_plexi_fails() {
        // `gh` ships in the embedded registry. With a runner that simulates
        // gh's real behavior (no native --plexi), the probe should fall
        // through to Tier 2 and resolve the registry descriptor.
        let mock = no_plexi_runner();
        let code = probe(&mock, "gh", &[]);
        assert_eq!(code, 0, "registry fallback should succeed for `gh`");
    }

    #[test]
    fn probe_no_registry_flag_skips_fallback() {
        // Same setup, but registry disabled. Should fail because Tier 1
        // fell through and Tier 2 is gated off.
        let mock = no_plexi_runner();
        let opts = ProbeOptions { use_registry: false, use_crawl: false };
        let code = probe_with_options(&mock, "gh", &[], &opts);
        assert_eq!(code, 1, "without registry or crawl, gh has no descriptor");
    }

    #[test]
    fn probe_surfaces_nonzero_exit_for_unknown_cli_with_no_registry_entry() {
        // No native --plexi, no registry hit → non-zero exit.
        let mock = no_plexi_runner();
        let code = probe(&mock, "nonexistent-cli-zzz", &[]);
        assert_eq!(code, 1);
    }

    #[test]
    fn probe_skips_registry_when_user_args_provided() {
        // Registry descriptors describe the bare CLI; subcommand invocations
        // shouldn't get a registry hit even if the CLI is registered.
        let mock = no_plexi_runner();
        let code = probe(&mock, "gh", &["pr", "create"]);
        assert_eq!(
            code, 1,
            "registry fallback only applies when no user args are passed"
        );
    }
}
