use clap::{Parser, Subcommand};
use plexi_distribution::{
    Error, Result,
    package::Package,
    release,
    transaction::{self, InstallOptions},
};
use std::{ffi::OsString, path::PathBuf};

#[derive(Parser)]
#[command(about = "Install, verify, recover, or roll back a Plexi package")]
struct Args {
    #[arg(long, default_value = "stable")]
    channel: String,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    package: Option<PathBuf>,
    #[arg(long, env = "PLEXI_INSTALL_DIR")]
    install_dir: Option<PathBuf>,
    #[arg(long, env = "PLEXI_BIN_DIR")]
    bin_dir: Option<PathBuf>,
    #[arg(long)]
    applications_dir: Option<PathBuf>,
    #[arg(long)]
    install_only: bool,
    #[arg(long)]
    dry_run: bool,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    Launch {
        #[arg(long)]
        receipt: PathBuf,
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    Restart {
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        wait_pid: u32,
    },
    Rollback {
        #[arg(long)]
        receipt: PathBuf,
    },
    Remove {
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        wait_pid: Option<u32>,
    },
    Verify {
        package: PathBuf,
    },
}

struct Logger;
impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            eprintln!("{}", r.args());
        }
    }
    fn flush(&self) {}
}
static LOGGER: Logger = Logger;

fn run(args: Args) -> Result<i32> {
    match args.command {
        Some(Action::Launch { receipt, args }) => return transaction::launch(&receipt, &args),
        Some(Action::Restart { receipt, wait_pid }) => {
            transaction::wait_for_parent(wait_pid)?;
            let current = transaction::read_receipt(&receipt)?
                .ok_or_else(|| Error::Invalid("installation disappeared during restart".into()))?;
            transaction::launch_ready(&current)?;
            return Ok(0);
        }
        Some(Action::Rollback { receipt }) => {
            let r = transaction::rollback(&receipt)?;
            println!("Restored {}. Restart Plexi to use it.", r.active.tag);
            return Ok(0);
        }
        Some(Action::Remove { receipt, wait_pid }) => {
            if let Some(pid) = wait_pid {
                transaction::wait_for_parent(pid)?;
            }
            transaction::uninstall(&receipt)?;
            println!("Removed the managed installation. User data retained.");
            return Ok(0);
        }
        Some(Action::Verify { package }) => {
            Package::load(&package)?.validate()?;
            return Ok(0);
        }
        None => {}
    }
    let channel = release::normalized_channel(&args.channel);
    let mut options = InstallOptions::for_channel(channel)?;
    if let Some(path) = args.install_dir {
        options.root =
            std::path::absolute(path.join(channel)).map_err(|e| Error::Invalid(e.to_string()))?;
    }
    if let Some(path) = args.bin_dir {
        options.bin_dir = path;
    }
    if let Some(path) = args.applications_dir {
        options.applications_dir = path;
    }
    let temp;
    let package_path = if let Some(path) = args.package {
        path
    } else {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(15))
            .build();
        let endpoint =
            std::env::var("PLEXI_RELEASES_URL").unwrap_or_else(|_| release::RELEASES_URL.into());
        let releases = release::fetch(&agent, &endpoint)?;
        let platform = release::platform()?;
        let selected = if let Some(tag) = &args.tag {
            releases.iter().find(|r| {
                &r.tag_name == tag
                    && !r.draft
                    && release::accepts(channel, &r.tag_name, r.prerelease)
            })
        } else {
            release::select(&releases, channel, &platform, None)
        }
        .ok_or_else(|| {
            Error::Invalid(format!(
                "no complete published release for {channel} on {platform}"
            ))
        })?;
        let asset = release::archive_name(&platform, channel);
        for name in [&asset, &format!("{asset}.sha256")] {
            if !selected.assets.iter().any(|a| &a.name == name) {
                return Err(Error::Invalid(format!(
                    "{} is missing {name}",
                    selected.tag_name
                )));
            }
        }
        println!("Plexi {} ({channel}, {platform})", selected.tag_name);
        println!("Installation: {}", options.root.display());
        if args.dry_run {
            return Ok(0);
        }
        temp = tempfile::tempdir()
            .map_err(|e| Error::Invalid(format!("create download directory: {e}")))?;
        let base = std::env::var("PLEXI_RELEASE_BASE_URL")
            .unwrap_or_else(|_| release::DOWNLOAD_URL.into());
        release::download_package(
            &agent,
            &base,
            &selected.tag_name,
            &platform,
            channel,
            temp.path(),
        )?;
        temp.path().join("package")
    };
    let package = Package::load(&package_path)?;
    package.validate()?;
    if args
        .tag
        .as_ref()
        .is_some_and(|tag| tag != &package.manifest.tag)
    {
        return Err(Error::Invalid(
            "downloaded package tag differs from requested tag".into(),
        ));
    }
    if args.dry_run {
        println!("Verified {}", package.root.display());
        return Ok(0);
    }
    let receipt = transaction::install(&package, options)?;
    println!(
        "Installed {} ({})",
        receipt.active.tag, receipt.active.build_id
    );
    println!(
        "Launch: \"{}\" host start",
        receipt.active.executable().display()
    );
    println!(
        "Command directory: {} (open a new shell to refresh PATH)",
        receipt.bin_dir.display()
    );
    #[cfg(unix)]
    println!(
        "For this shell: export PATH='{}':\"$PATH\"",
        receipt.bin_dir.to_string_lossy().replace('\'', "'\\''")
    );
    if !args.install_only {
        transaction::launch_ready(&receipt)?;
        println!("Plexi is ready.");
    }
    Ok(0)
}

fn main() {
    log::set_logger(&LOGGER).expect("installer logger");
    log::set_max_level(log::LevelFilter::Info);
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            let sidecar = exe.with_extension("install.json");
            if sidecar.is_file() {
                let result = std::fs::read(sidecar)
                    .map_err(|e| Error::Invalid(e.to_string()))
                    .and_then(|bytes| Ok(serde_json::from_slice::<PathBuf>(&bytes)?))
                    .and_then(|root| {
                        transaction::launch(&root, &std::env::args_os().skip(1).collect::<Vec<_>>())
                    });
                match result {
                    Ok(code) => std::process::exit(code),
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
    }
    match run(Args::parse()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
