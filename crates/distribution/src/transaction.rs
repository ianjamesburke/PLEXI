use crate::{
    Error, IoContext, Result,
    package::{Package, hash_file},
    release,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const RECEIPT: &str = "installation.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generation {
    pub path: PathBuf,
    pub build_id: String,
    pub tag: String,
    pub executable: String,
}
impl Generation {
    pub fn executable(&self) -> PathBuf {
        self.path.join(&self.executable)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnedPath {
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub link_target: Option<PathBuf>,
}
impl OwnedPath {
    pub fn matches(&self) -> Result<bool> {
        if let Some(target) = &self.link_target {
            return Ok(self.path.is_symlink()
                && fs::read_link(&self.path).context("read owned symlink")? == *target);
        }
        Ok(!self.path.is_symlink() && self.path.is_file() && hash_file(&self.path)? == self.sha256)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathEdit {
    pub path: PathBuf,
    pub block: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: u32,
    pub channel: String,
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub applications_dir: PathBuf,
    pub active: Generation,
    pub previous: Option<Generation>,
    pub integrations: Vec<OwnedPath>,
    #[serde(default)]
    pub path_edits: Vec<PathEdit>,
    #[serde(default)]
    pub windows_path_added: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Change {
    path: PathBuf,
    before: Option<PathBuf>,
    staged: PathBuf,
    after_hash: String,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    link_target: Option<PathBuf>,
    #[serde(default)]
    before_link: Option<PathBuf>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    before: Option<Receipt>,
    after: Receipt,
    changes: Vec<Change>,
    #[serde(default)]
    windows_path: Option<WindowsPathChange>,
}
#[derive(Debug, Serialize, Deserialize)]
struct WindowsPathChange {
    before: String,
    after: String,
}

pub fn default_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PLEXI_DISTRIBUTION_HOME") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    let base = dirs::data_local_dir().map(|p| p.join("Plexi"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".local/share")))
        .map(|p| p.join("plexi"));
    base.ok_or_else(|| Error::Invalid("cannot resolve user installation directory".into()))
}

pub fn registry_path(channel: &str) -> Result<PathBuf> {
    release::validate_channel(channel)?;
    Ok(default_root()?
        .join("installations")
        .join(format!("{channel}.json")))
}

pub fn read_receipt(root: &Path) -> Result<Option<Receipt>> {
    let path = root.join(RECEIPT);
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                action: format!("read {}", path.display()),
                source,
            });
        }
    };
    let receipt: Receipt = serde_json::from_slice(&data)?;
    let canonical = root.canonicalize().context("resolve receipt root")?;
    if receipt.schema != 1
        || receipt.root != canonical
        || !receipt
            .active
            .path
            .starts_with(canonical.join("generations"))
    {
        return Err(Error::Invalid(format!(
            "invalid installation receipt at {}",
            path.display()
        )));
    }
    release::validate_channel(&receipt.channel)?;
    crate::package::relative(&receipt.active.executable)?;
    Ok(Some(receipt))
}

pub fn find_installation(channel: &str) -> Result<Option<Receipt>> {
    let channel = release::normalized_channel(channel);
    let index = registry_path(channel)?;
    match fs::read(index) {
        Ok(data) => {
            let root: PathBuf = serde_json::from_slice(&data)?;
            read_receipt(&root)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            read_receipt(&default_root()?.join(channel))
        }
        Err(source) => Err(Error::Io {
            action: "read installation registry".into(),
            source,
        }),
    }
}

pub fn installations() -> Result<Vec<Receipt>> {
    let registry = default_root()?.join("installations");
    if !registry.exists() {
        return Ok(vec![]);
    }
    let mut receipts = Vec::new();
    for entry in fs::read_dir(registry).context("list installation registry")? {
        let entry = entry.context("read installation registry entry")?;
        if entry.path().extension().is_some_and(|e| e == "json") {
            let root: PathBuf = serde_json::from_slice(
                &fs::read(entry.path()).context("read installation registration")?,
            )?;
            if let Some(receipt) = read_receipt(&root)? {
                receipts.push(receipt);
            }
        }
    }
    receipts.sort_by(|a, b| a.channel.cmp(&b.channel));
    Ok(receipts)
}

pub fn installed_for_executable(exe: &Path) -> Result<Option<Receipt>> {
    let Some(package) = Package::at_executable(exe)? else {
        return Ok(None);
    };
    let Some(generations) = package
        .root
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "generations"))
    else {
        return Ok(None);
    };
    let root = generations
        .parent()
        .ok_or_else(|| Error::Invalid("generation has no installation root".into()))?;
    read_receipt(root)
}

pub fn lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root).context("create installation root")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("install.lock"))
        .context("open installation lock")?;
    file.try_lock().map_err(|e| {
        Error::Invalid(format!(
            "another installer is using {}: {e}",
            root.display()
        ))
    })?;
    Ok(file)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("destination has no parent".into()))?;
    fs::create_dir_all(parent).context(format!("create {}", parent.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).context("stage atomic write")?;
    if let Ok(metadata) = fs::metadata(path) {
        file.as_file()
            .set_permissions(metadata.permissions())
            .context("preserve destination permissions")?;
    }
    file.write_all(bytes).context("write staged file")?;
    file.as_file().sync_all().context("sync staged file")?;
    file.persist(path).map_err(|e| Error::Io {
        action: format!("activate {}", path.display()),
        source: e.error,
    })?;
    sync_dir(parent)
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .context("open directory for sync")?
        .sync_all()
        .context("sync directory")?;
    #[cfg(windows)]
    let _ = path;
    Ok(())
}
fn remove_if_present(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let deadline = Instant::now() + Duration::from_secs(10);
    #[cfg(not(windows))]
    return match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            action: format!("remove {}", path.display()),
            source,
        }),
    };
    #[cfg(windows)]
    loop {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            #[cfg(windows)]
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(source) => {
                return Err(Error::Io {
                    action: format!("remove {}", path.display()),
                    source,
                });
            }
        }
    }
}

fn prepare_file(root: &Path, path: &Path, bytes: &[u8], index: usize) -> Result<Change> {
    let work = root.join("transaction");
    fs::create_dir_all(&work).context("prepare transaction directory")?;
    // Refuse to follow someone else's symlink when taking ownership.
    if path.is_symlink() {
        return Err(Error::Invalid(format!(
            "refusing to replace unowned symlink {}",
            path.display()
        )));
    }
    let before = if path.exists() {
        let backup = work.join(format!("{index}.before"));
        fs::copy(path, &backup).context("back up integration")?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&backup)
            .context("open integration backup")?
            .sync_all()
            .context("sync integration backup")?;
        Some(backup)
    } else {
        None
    };
    let staged = work.join(format!("{index}.after"));
    atomic_write(&staged, bytes)?;
    let after_hash = hash_file(&staged)?;
    Ok(Change {
        path: path.to_path_buf(),
        before,
        staged,
        after_hash,
        executable: false,
        link_target: None,
        before_link: None,
    })
}

fn executable_permission(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .context("set launcher executable permission")?;
    }
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

fn apply_changes(changes: &[Change]) -> Result<()> {
    for change in changes {
        if let Some(target) = &change.link_target {
            atomic_link(&change.path, target)?;
        } else {
            let bytes = fs::read(&change.staged).context("read staged integration")?;
            atomic_write(&change.path, &bytes)?;
            if change.executable {
                executable_permission(&change.path)?;
            }
        }
    }
    Ok(())
}

pub fn recover(root: &Path) -> Result<()> {
    let path = root.join("transaction.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::Io {
                action: "read transaction journal".into(),
                source,
            });
        }
    };
    let journal: Journal = serde_json::from_slice(&bytes)?;
    log::info!(
        "distribution: recovering interrupted activation in {}",
        root.display()
    );
    for change in journal.changes.iter().rev() {
        let owned = OwnedPath {
            path: change.path.clone(),
            sha256: change.after_hash.clone(),
            link_target: change.link_target.clone(),
        };
        if owned.matches()? {
            if let Some(target) = &change.before_link {
                atomic_link(&change.path, target)?;
            } else if let Some(before) = &change.before {
                atomic_write(
                    &change.path,
                    &fs::read(before).context("read integration backup")?,
                )?;
            } else {
                remove_if_present(&change.path)?;
            }
        }
    }
    #[cfg(windows)]
    if let Some(change) = &journal.windows_path
        && read_windows_path()? == change.after
    {
        write_windows_path(&change.before)?;
    }
    if let Some(before) = journal.before {
        write_json(&root.join(RECEIPT), &before)?;
    } else {
        remove_if_present(&root.join(RECEIPT))?;
    }
    remove_if_present(&path)?;
    sync_dir(root)
}

pub struct InstallOptions {
    pub channel: String,
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub applications_dir: PathBuf,
}
impl InstallOptions {
    pub fn for_channel(channel: &str) -> Result<Self> {
        let channel = release::normalized_channel(channel);
        release::validate_channel(channel)?;
        if let Some(receipt) = find_installation(channel)? {
            return Ok(Self {
                channel: channel.into(),
                root: receipt.root,
                bin_dir: receipt.bin_dir,
                applications_dir: receipt.applications_dir,
            });
        }
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Invalid("cannot resolve home directory".into()))?;
        #[cfg(windows)]
        let bin_dir = default_root()?.join("bin");
        #[cfg(not(windows))]
        let bin_dir = home.join(".local/bin");
        Ok(Self {
            channel: channel.into(),
            root: default_root()?.join(channel),
            bin_dir,
            applications_dir: home.join("Applications"),
        })
    }
}

fn copy_package(package: &Package, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).context("create package generation")?;
    for name in package
        .manifest
        .files
        .keys()
        .chain(std::iter::once(&"package.json".to_string()))
    {
        let path = destination.join(crate::package::relative(name)?);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| Error::Invalid("package entry has no parent".into()))?,
        )
        .context("create package directory")?;
        fs::copy(package.root.join(name), &path).context(format!("copy package file {name}"))?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .context("open package file for sync")?
            .sync_all()
            .context("sync package file")?;
    }
    fn sync_tree(dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir).context("read package directories for sync")? {
            let entry = entry.context("read package directory")?;
            if entry
                .file_type()
                .context("inspect package directory")?
                .is_dir()
            {
                sync_tree(&entry.path())?;
            }
        }
        sync_dir(dir)
    }
    sync_tree(destination)
}

fn verify_target(generation: &Generation) -> Result<()> {
    let mut command = Command::new(generation.executable());
    command.arg("--distribution-check");
    clean_environment(&mut command);
    let output = command.output().context("verify installed runtime")?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "installed runtime check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let identity: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if identity["build_id"].as_str() != Some(&generation.build_id)
        || identity["tag"].as_str() != Some(&generation.tag)
    {
        return Err(Error::Invalid(
            "installed build identity differs from package".into(),
        ));
    }
    Ok(())
}

pub fn clean_environment(command: &mut Command) {
    clean_environment_from(command, std::env::vars_os());
}

fn clean_environment_from(
    command: &mut Command,
    variables: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) {
    for (key, value) in variables {
        // A bootstrap invoked from a Plexi terminal must restore the user's
        // original zsh directory before dropping the pane's integration marker.
        if key == "PLEXI_ORIG_ZDOTDIR" {
            command.env("ZDOTDIR", value);
        }
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
}
#[cfg(unix)]
fn quote_shell(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn integration_files(package: &Package, receipt: &Receipt) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let name = release::command_name(&receipt.channel);
    let mut files = Vec::new();
    #[cfg(unix)]
    {
        let installer = package.root.join("plexi-installer");
        let script = format!(
            "#!/bin/sh\nexec {} launch --receipt {} -- \"$@\"\n",
            quote_shell(&installer),
            quote_shell(&receipt.root)
        );
        files.push((receipt.bin_dir.join(&name), script.into_bytes()));
    }
    #[cfg(windows)]
    {
        let launcher = receipt.bin_dir.join(format!("{name}.exe"));
        // This is the schema-1 launcher, not another host executable. It remains
        // stable while versioned application executables can be running.
        if launcher.exists()
            && !receipt
                .integrations
                .iter()
                .any(|p| p.path == launcher && p.matches().unwrap_or(false))
        {
            return Err(Error::Invalid(format!(
                "refusing to use unowned Windows launcher {}",
                launcher.display()
            )));
        }
        if !launcher.exists() {
            files.push((
                launcher,
                fs::read(package.root.join("plexi-installer.exe"))
                    .context("read Windows launcher")?,
            ));
        }
        files.push((
            receipt.bin_dir.join(format!("{name}.install.json")),
            serde_json::to_vec(&receipt.root)?,
        ));
    }
    files.push((
        registry_path(&receipt.channel)?,
        serde_json::to_vec(&receipt.root)?,
    ));
    #[cfg(target_os = "linux")]
    {
        let desktop = default_root()?
            .parent()
            .ok_or_else(|| Error::Invalid("XDG root has no parent".into()))?
            .join("applications")
            .join(format!("{name}.desktop"));
        let exec = receipt
            .bin_dir
            .join(&name)
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$");
        let icon = package.resources()?.join("app-icon.png");
        files.push((desktop, format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{exec}\" host start\nIcon={}\nTerminal=false\nCategories=Development;System;\n", display_name(&receipt.channel), icon.display()).into_bytes()));
    }
    Ok(files)
}

/// Recognize only the old release installer's duplicated command: its bytes
/// must match a host at the exact historical payload path for this channel.
fn legacy_release_command(path: &Path, receipt: &Receipt) -> Result<bool> {
    let name = release::command_name(&receipt.channel);
    let expected = receipt.bin_dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name
    });
    if path != expected || path.is_symlink() || !path.is_file() {
        return Ok(false);
    }
    let parent = receipt
        .root
        .parent()
        .ok_or_else(|| Error::Invalid("installation root has no parent".into()))?;
    let legacy = parent.join(if receipt.channel == "stable" && !cfg!(windows) {
        "main"
    } else {
        &receipt.channel
    });
    let checksum = hash_file(path)?;
    for relative in ["plexi", "plexi.exe", "Plexi.app/Contents/MacOS/plexi"] {
        let payload = legacy.join(relative);
        if payload.is_file() && !payload.is_symlink() && hash_file(&payload)? == checksum {
            log::info!(
                "distribution: adopting verified legacy release command {}; retaining legacy payload at {}",
                path.display(),
                legacy.display()
            );
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn display_name(channel: &str) -> String {
    match channel {
        "stable" | "main" => "Plexi".into(),
        "alpha" => "Plexi Alpha".into(),
        "beta" => "Plexi Beta".into(),
        c if c.starts_with("pr-") => format!("Plexi PR{}", &c[3..]),
        c => format!("Plexi {}{}", c[..1].to_uppercase(), &c[1..]),
    }
}

pub fn install(package: &Package, options: InstallOptions) -> Result<Receipt> {
    install_checked(package, options, None)
}

/// Refuse an update if another installer or rollback changed the selected base
/// while the release was downloading. The comparison runs under the channel lock.
pub fn update(package: &Package, options: InstallOptions, expected_build: &str) -> Result<Receipt> {
    install_checked(package, options, Some(expected_build))
}

fn install_checked(
    package: &Package,
    options: InstallOptions,
    expected_build: Option<&str>,
) -> Result<Receipt> {
    package.validate()?;
    if package.manifest.channel != options.channel
        || package.manifest.platform != release::platform()?
    {
        return Err(Error::Invalid(
            "package channel or platform does not match the requested installation".into(),
        ));
    }
    let mut options = options;
    for path in [&mut options.bin_dir, &mut options.applications_dir] {
        fs::create_dir_all(&*path).context("create integration directory")?;
        *path = path
            .canonicalize()
            .context("resolve integration directory")?;
    }
    let _lock = lock(&options.root)?;
    let _integration_lock = lock(&default_root()?.join("integration-lock"))?;
    let root = options
        .root
        .canonicalize()
        .context("canonicalize installation root")?;
    recover(&root)?;
    let before = read_receipt(&root)?;
    if let Some(expected) = expected_build {
        check_update_base(before.as_ref(), expected)?;
    }
    let generations = root.join("generations");
    fs::create_dir_all(&generations).context("create generations directory")?;
    let digest = hash_file(&package.root.join("package.json"))?;
    let destination = generations.join(&digest);
    if !destination.exists() {
        let stage = tempfile::Builder::new()
            .prefix(".stage-")
            .tempdir_in(&generations)
            .context("stage package beside destination")?;
        copy_package(package, stage.path())?;
        Package::load(stage.path())?.validate()?;
        fs::rename(stage.path(), &destination).context("commit verified package generation")?;
        sync_dir(&generations)?;
    }
    let installed = Package::load(&destination)?;
    installed.validate()?;
    let generation = Generation {
        path: destination,
        build_id: installed.manifest.build_id.clone(),
        tag: installed.manifest.tag.clone(),
        executable: installed.manifest.executable.clone(),
    };
    verify_target(&generation)?;
    let previous = before.as_ref().and_then(|r| {
        if r.active.path == generation.path {
            r.previous.clone()
        } else {
            Some(r.active.clone())
        }
    });
    let mut receipt = Receipt {
        schema: 1,
        channel: options.channel,
        root: root.clone(),
        bin_dir: options.bin_dir,
        applications_dir: options.applications_dir,
        active: generation,
        previous,
        integrations: before
            .as_ref()
            .map(|r| r.integrations.clone())
            .unwrap_or_default(),
        path_edits: vec![],
        windows_path_added: before.as_ref().is_some_and(|r| r.windows_path_added),
    };
    let files = integration_files(&installed, &receipt)?;
    let mut changes = Vec::new();
    for (path, bytes) in files {
        if path.exists() || path.is_symlink() {
            let owned = before.as_ref().is_some_and(|r| {
                r.integrations
                    .iter()
                    .any(|p| p.path == path && p.matches().unwrap_or(false))
            });
            if !owned && !legacy_release_command(&path, &receipt)? {
                return Err(Error::Invalid(format!(
                    "refusing to overwrite unowned integration {}",
                    path.display()
                )));
            }
        }
        let mut change = prepare_file(&root, &path, &bytes, changes.len())?;
        change.executable = path
            == receipt
                .bin_dir
                .join(release::command_name(&receipt.channel));
        receipt.integrations.retain(|p| p.path != path);
        receipt.integrations.push(OwnedPath {
            path,
            sha256: change.after_hash.clone(),
            link_target: None,
        });
        changes.push(change);
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = receipt
            .active
            .executable()
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::Invalid("Mac package has no app bundle".into()))?;
        let path = receipt
            .applications_dir
            .join(format!("{}.app", display_name(&receipt.channel)));
        let before_link = if path.exists() || path.is_symlink() {
            if !before.as_ref().is_some_and(|r| {
                r.integrations
                    .iter()
                    .any(|p| p.path == path && p.matches().unwrap_or(false))
            }) {
                return Err(Error::Invalid(format!(
                    "refusing to replace unowned bundle {}",
                    path.display()
                )));
            }
            Some(fs::read_link(&path).context("read existing Applications link")?)
        } else {
            None
        };
        changes.push(Change {
            path: path.clone(),
            before: None,
            staged: PathBuf::new(),
            after_hash: String::new(),
            executable: false,
            link_target: Some(bundle.clone()),
            before_link,
        });
        receipt.integrations.retain(|p| p.path != path);
        receipt.integrations.push(OwnedPath {
            path,
            sha256: String::new(),
            link_target: Some(bundle),
        });
    }
    #[cfg(unix)]
    prepare_path_registration(&mut receipt, &mut changes)?;
    let windows_path = prepare_windows_integration(&mut receipt, &mut changes)?;
    let journal = Journal {
        before,
        after: receipt.clone(),
        changes,
        windows_path,
    };
    write_json(&root.join("transaction.json"), &journal)?;
    let result = (|| {
        apply_changes(&journal.changes)?;
        #[cfg(windows)]
        if let Some(change) = &journal.windows_path {
            write_windows_path(&change.after)?;
        }
        write_json(&root.join(RECEIPT), &receipt)?;
        verify_target(&receipt.active)?;
        remove_if_present(&root.join("transaction.json"))?;
        sync_dir(&root)
    })();
    if let Err(error) = result {
        recover(&root)?;
        return Err(error);
    }
    log::info!(
        "distribution: activated channel={} build={} root={}",
        receipt.channel,
        receipt.active.build_id,
        root.display()
    );
    Ok(receipt)
}

fn check_update_base(receipt: Option<&Receipt>, expected: &str) -> Result<()> {
    if receipt.is_some_and(|r| r.active.build_id == expected) {
        Ok(())
    } else {
        Err(Error::Invalid(
            "installation changed while downloading; run update again".into(),
        ))
    }
}

fn atomic_link(path: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .ok_or_else(|| Error::Invalid("symlink has no parent".into()))?;
        fs::create_dir_all(parent).context("create integration parent")?;
        let temp = tempfile::Builder::new()
            .prefix(".plexi-link-")
            .tempdir_in(parent)
            .context("stage integration link")?;
        let staged = temp.path().join("link");
        std::os::unix::fs::symlink(target, &staged).context("create integration link")?;
        fs::rename(staged, path).context("activate integration link")?;
        sync_dir(parent)
    }
    #[cfg(windows)]
    {
        Err(Error::Invalid(format!(
            "symlink integration unsupported: {} -> {}",
            path.display(),
            target.display()
        )))
    }
}

pub fn remove_owned(owned: &OwnedPath) -> Result<()> {
    if !owned.path.exists() && !owned.path.is_symlink() {
        return Ok(());
    }
    if !owned.matches()? {
        return Err(Error::Invalid(format!(
            "preserving changed or unowned file {}",
            owned.path.display()
        )));
    }
    remove_if_present(&owned.path)
}

#[cfg(unix)]
fn prepare_path_registration(receipt: &mut Receipt, changes: &mut Vec<Change>) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| Error::Invalid("no home for shell configuration".into()))?;
    let marker = format!("# Plexi command path: {}", receipt.bin_dir.display());
    let block = format!(
        "\n{marker}\nexport PATH={}:\"$PATH\"\n# End Plexi command path\n",
        quote_shell(&receipt.bin_dir)
    );
    let mut paths = vec![
        home.join(".profile"),
        home.join(".zprofile"),
        home.join(".zshrc"),
        home.join(".bashrc"),
    ];
    if home.join(".bash_profile").exists() {
        paths.push(home.join(".bash_profile"));
    } else if home.join(".bash_login").exists() {
        paths.push(home.join(".bash_login"));
    }
    for path in paths {
        let existing = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(Error::Io {
                    action: format!("read shell profile {}", path.display()),
                    source,
                });
            }
        };
        if !existing.contains(&block) {
            if existing.contains(&marker) {
                return Err(Error::Invalid(format!(
                    "Plexi PATH block was edited in {}; update it manually",
                    path.display()
                )));
            }
            // Follow a user's dotfile symlink to edit its contents atomically,
            // retaining the symlink itself and preserving all unrelated text.
            let destination = if path.is_symlink() {
                path.canonicalize().context("resolve shell profile link")?
            } else {
                path.clone()
            };
            changes.push(prepare_file(
                &receipt.root,
                &destination,
                format!("{existing}{block}").as_bytes(),
                changes.len(),
            )?);
        }
        receipt.path_edits.push(PathEdit {
            path,
            block: block.clone(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn remove_path_registration(receipt: &Receipt) -> Result<()> {
    let registry = default_root()?.join("installations");
    if registry.is_dir() {
        for entry in fs::read_dir(registry).context("list other installations")? {
            let entry = entry.context("read installation registry entry")?;
            if entry.path().extension().is_some_and(|e| e == "json") {
                let root: PathBuf = serde_json::from_slice(
                    &fs::read(entry.path()).context("read other installation")?,
                )?;
                if root != receipt.root
                    && read_receipt(&root)?.is_some_and(|r| r.bin_dir == receipt.bin_dir)
                {
                    return Ok(());
                }
            }
        }
    }
    for edit in &receipt.path_edits {
        if !edit.path.exists() {
            continue;
        }
        let text = fs::read_to_string(&edit.path).context("read shell PATH configuration")?;
        if text.contains(&edit.block) {
            let destination = edit
                .path
                .canonicalize()
                .context("resolve shell PATH configuration")?;
            atomic_write(&destination, text.replace(&edit.block, "").as_bytes())?;
        }
    }
    Ok(())
}

pub fn uninstall(root: &Path) -> Result<()> {
    let _lock = lock(root)?;
    let _integration_lock = lock(&default_root()?.join("integration-lock"))?;
    recover(root)?;
    let receipt = read_receipt(root)?
        .ok_or_else(|| Error::Invalid("no managed installation found; no files removed".into()))?;
    // Preflight every owned integration before deleting any of them.
    for owned in &receipt.integrations {
        if (owned.path.exists() || owned.path.is_symlink()) && !owned.matches()? {
            return Err(Error::Invalid(format!(
                "preserving changed integration {}",
                owned.path.display()
            )));
        }
    }
    #[cfg(windows)]
    windows_remove(&receipt)?;
    #[cfg(unix)]
    remove_path_registration(&receipt)?;
    for owned in &receipt.integrations {
        remove_owned(owned)?;
    }
    for entry in fs::read_dir(root.join("generations")).context("list retained generations")? {
        let entry = entry.context("read generation")?;
        if entry.path().join("package.json").is_file() {
            let package = Package::load(&entry.path())?;
            package.validate()?;
            fs::remove_dir_all(&package.root)
                .context(format!("remove generation {}", package.root.display()))?;
        }
    }
    remove_if_present(&root.join(RECEIPT))?;
    log::info!(
        "distribution: removed managed channel={} root={}; user data retained",
        receipt.channel,
        root.display()
    );
    Ok(())
}

pub fn rollback(root: &Path) -> Result<Receipt> {
    // install takes the same lock, performs the same checks and retains the
    // displaced generation, so rolling forward again uses the same path.
    let receipt =
        read_receipt(root)?.ok_or_else(|| Error::Invalid("no managed installation".into()))?;
    let previous = receipt
        .previous
        .ok_or_else(|| Error::Invalid("no retained rollback generation".into()))?;
    let package = Package::load(&previous.path)?;
    install(
        &package,
        InstallOptions {
            channel: receipt.channel,
            root: receipt.root,
            bin_dir: receipt.bin_dir,
            applications_dir: receipt.applications_dir,
        },
    )
}

pub fn launch(root: &Path, args: &[std::ffi::OsString]) -> Result<i32> {
    if let Ok(_lock) = lock(root) {
        recover(root)?;
    }
    let receipt = read_receipt(root)?.ok_or_else(|| {
        Error::Invalid("installation is incomplete; run the installer again".into())
    })?;
    // Contextual bare-command routing is limited to an actual Plexi pane.
    if receipt.channel == "stable"
        && std::env::var("PLEXI_RUNNING").as_deref() == Ok("1")
        && let Ok(channel) = std::env::var("PLEXI_CHANNEL")
        && channel != "stable"
        && channel != "main"
        && let Some(other) = find_installation(&channel)?
    {
        return launch(&other.root, args);
    }
    let mut command = Command::new(receipt.active.executable());
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let source = command.exec();
        Err(Error::Io {
            action: "launch installed Plexi".into(),
            source,
        })
    }
    #[cfg(windows)]
    {
        Ok(command
            .status()
            .context("launch installed Plexi")?
            .code()
            .unwrap_or(1))
    }
}

pub fn launch_ready(receipt: &Receipt) -> Result<()> {
    let mut command = Command::new(receipt.active.executable());
    command.args(["host", "start"]);
    clean_environment(&mut command);
    let status = command.status().context("start installed host")?;
    if !status.success() {
        return Err(Error::Invalid(
            "installed host did not report ready; run the printed absolute host start command"
                .into(),
        ));
    }
    Ok(())
}

pub fn wait_for_parent(pid: u32) -> Result<()> {
    let start = Instant::now();
    while process_alive(pid) {
        if start.elapsed() > Duration::from_secs(120) {
            return Err(Error::Invalid(format!("process {pid} has not exited")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let result = WaitForSingleObject(handle, 0) == 258;
        CloseHandle(handle);
        result
    }
}

pub fn schedule_restart(receipt: &Receipt, pid: u32) -> Result<()> {
    let installer = receipt.active.path.join(if cfg!(windows) {
        "plexi-installer.exe"
    } else {
        "plexi-installer"
    });
    let mut command = Command::new(installer);
    command
        .args(["restart", "--receipt"])
        .arg(&receipt.root)
        .arg("--wait-pid")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    clean_environment(&mut command);
    command
        .spawn()
        .context("schedule restart of installed generation")?;
    Ok(())
}

#[cfg(not(windows))]
fn prepare_windows_integration(
    _receipt: &mut Receipt,
    _changes: &mut Vec<Change>,
) -> Result<Option<WindowsPathChange>> {
    Ok(None)
}

#[cfg(windows)]
fn powershell(script: &str, env: &[(&str, &std::ffi::OsStr)]) -> Result<std::process::Output> {
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().context("run Windows integration")?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "Windows integration failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output)
}
#[cfg(windows)]
fn read_windows_path() -> Result<String> {
    let output = powershell(
        "$ErrorActionPreference='Stop'; ConvertTo-Json -Compress -InputObject ([string][Environment]::GetEnvironmentVariable('Path','User'))",
        &[],
    )?;
    Ok(serde_json::from_slice(&output.stdout)?)
}
#[cfg(windows)]
fn write_windows_path(path: &str) -> Result<()> {
    powershell(
        "$ErrorActionPreference='Stop'; [Environment]::SetEnvironmentVariable('Path',$env:PLEXI_REGISTER_PATH,'User')",
        &[("PLEXI_REGISTER_PATH", std::ffi::OsStr::new(path))],
    )?;
    Ok(())
}
#[cfg(windows)]
fn prepare_windows_integration(
    receipt: &mut Receipt,
    changes: &mut Vec<Change>,
) -> Result<Option<WindowsPathChange>> {
    let before = read_windows_path()?;
    let bin = receipt.bin_dir.to_string_lossy();
    let exists = before.split(';').any(|p| p.eq_ignore_ascii_case(&bin));
    receipt.windows_path_added |= !exists;
    let registry = default_root()?.join("installations");
    if registry.is_dir() {
        for entry in fs::read_dir(registry).context("list shared PATH owners")? {
            let entry = entry.context("read shared PATH owner")?;
            if entry.path().extension().is_some_and(|e| e == "json") {
                let root: PathBuf = serde_json::from_slice(
                    &fs::read(entry.path()).context("read shared installation")?,
                )?;
                if read_receipt(&root)?
                    .is_some_and(|r| r.bin_dir == receipt.bin_dir && r.windows_path_added)
                {
                    receipt.windows_path_added = true;
                }
            }
        }
    }
    let after = if exists {
        before.clone()
    } else {
        format!("{bin};{before}")
    };
    let menu = dirs::data_dir()
        .ok_or_else(|| Error::Invalid("no Start Menu directory".into()))?
        .join("Microsoft/Windows/Start Menu/Programs");
    let path = menu.join(format!("{}.lnk", display_name(&receipt.channel)));
    if path.exists()
        && !receipt
            .integrations
            .iter()
            .any(|p| p.path == path && p.matches().unwrap_or(false))
    {
        return Err(Error::Invalid(format!(
            "unowned Start Menu shortcut {}",
            path.display()
        )));
    }
    let stage = receipt.root.join("transaction/menu.lnk");
    let launcher = receipt
        .bin_dir
        .join(format!("{}.exe", release::command_name(&receipt.channel)));
    powershell(
        "$ErrorActionPreference='Stop'; $w=New-Object -ComObject WScript.Shell; $s=$w.CreateShortcut($env:PLEXI_REGISTER_LINK); $s.TargetPath=$env:PLEXI_REGISTER_EXE; $s.Arguments='host start'; $s.Save()",
        &[
            ("PLEXI_REGISTER_LINK", stage.as_os_str()),
            ("PLEXI_REGISTER_EXE", launcher.as_os_str()),
        ],
    )?;
    let change = prepare_file(
        &receipt.root,
        &path,
        &fs::read(&stage).context("read staged shortcut")?,
        changes.len(),
    )?;
    receipt.integrations.retain(|p| p.path != path);
    receipt.integrations.push(OwnedPath {
        path,
        sha256: change.after_hash.clone(),
        link_target: None,
    });
    changes.push(change);
    Ok(Some(WindowsPathChange { before, after }))
}
#[cfg(windows)]
fn windows_remove(receipt: &Receipt) -> Result<()> {
    if !receipt.windows_path_added {
        return Ok(());
    }
    if fs::read_dir(&receipt.bin_dir)
        .context("list shared Windows launchers")?
        .any(|e| {
            e.is_ok_and(|e| {
                e.path().extension().is_some_and(|x| x == "json")
                    && e.file_name()
                        != format!("{}.install.json", release::command_name(&receipt.channel))
                            .as_str()
            })
        })
    {
        return Ok(());
    }
    let before = read_windows_path()?;
    let after = before
        .split(';')
        .filter(|p| !p.eq_ignore_ascii_case(&receipt.bin_dir.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(";");
    write_windows_path(&after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_installer_cannot_lock_the_same_channel() {
        let dir = tempfile::tempdir().unwrap();
        let _lock = lock(dir.path()).unwrap();
        assert!(lock(dir.path()).is_err());
    }

    #[test]
    fn interrupted_activation_restores_the_previous_record_and_integration() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("bin/plexi");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        fs::write(&link, b"old launcher").unwrap();
        let change = prepare_file(dir.path(), &link, b"new launcher", 0).unwrap();
        let journal = Journal {
            before: None,
            after: fixture_receipt(dir.path()),
            changes: vec![change],
            windows_path: None,
        };
        write_json(&dir.path().join("transaction.json"), &journal).unwrap();
        apply_changes(&journal.changes).unwrap();
        write_json(&dir.path().join(RECEIPT), &journal.after).unwrap();
        recover(dir.path()).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"old launcher");
        assert!(!dir.path().join(RECEIPT).exists());
    }

    #[test]
    fn modified_integration_is_never_removed_as_owned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plexi");
        fs::write(&path, b"managed").unwrap();
        let owned = OwnedPath {
            path: path.clone(),
            sha256: crate::package::hash_file(&path).unwrap(),
            link_target: None,
        };
        fs::write(&path, b"unrelated development binary").unwrap();
        assert!(remove_owned(&owned).is_err());
        assert!(path.exists());
    }

    #[test]
    fn updates_cannot_overwrite_a_concurrent_upgrade_or_rollback() {
        let receipt = fixture_receipt(Path::new("/unused"));
        assert!(check_update_base(Some(&receipt), "new").is_ok());
        assert!(check_update_base(Some(&receipt), "old").is_err());
        assert!(check_update_base(None, "new").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_integration_changes_preserve_private_dotfile_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zshrc");
        fs::write(&path, b"#!/bin/sh\nuser shell configuration").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let change = prepare_file(
            dir.path(),
            &path,
            b"#!/bin/sh\nuser shell configuration plus PATH",
            0,
        )
        .unwrap();
        apply_changes(&[change]).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn launch_environment_restores_original_zsh_configuration() {
        let mut command = Command::new("unused");
        clean_environment_from(
            &mut command,
            [
                ("PLEXI_ORIG_ZDOTDIR".into(), "/user/zsh".into()),
                (
                    "ZDOTDIR".into(),
                    "/user/.plexi-beta/shell-integration/zsh".into(),
                ),
                ("PLEXI_CHANNEL".into(), "beta".into()),
            ]
            .into_iter(),
        );
        let edits: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            edits[std::ffi::OsStr::new("ZDOTDIR")],
            Some(std::ffi::OsStr::new("/user/zsh"))
        );
        assert_eq!(edits[std::ffi::OsStr::new("PLEXI_ORIG_ZDOTDIR")], None);
        assert_eq!(edits[std::ffi::OsStr::new("PLEXI_CHANNEL")], None);
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_windows_path_registration_restores_previous_value() {
        let dir = tempfile::tempdir().unwrap();
        let before = read_windows_path().unwrap();
        let after = format!("{};{before}", dir.path().display());
        let journal = Journal {
            before: None,
            after: fixture_receipt(dir.path()),
            changes: vec![],
            windows_path: Some(WindowsPathChange {
                before: before.clone(),
                after: after.clone(),
            }),
        };
        write_json(&dir.path().join("transaction.json"), &journal).unwrap();
        write_windows_path(&after).unwrap();
        let result = recover(dir.path());
        // Restore even if the assertion below needs to report a failure.
        let observed = read_windows_path().unwrap();
        write_windows_path(&before).unwrap();
        result.unwrap();
        assert_eq!(observed, before);
    }

    fn fixture_receipt(root: &Path) -> Receipt {
        Receipt {
            schema: 1,
            channel: "stable".into(),
            root: root.to_path_buf(),
            bin_dir: root.join("bin"),
            applications_dir: root.join("Applications"),
            active: Generation {
                path: root.join("generations/new"),
                build_id: "new".into(),
                tag: "v1.0.0".into(),
                executable: "plexi".into(),
            },
            previous: None,
            integrations: vec![],
            path_edits: vec![],
            windows_path_added: false,
        }
    }
}
