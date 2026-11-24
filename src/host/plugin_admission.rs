//! Which plugin code the host starts, and which install sources are accepted
//! (stint 1398).
//!
//! Until third-party installs open, Plexi runs only plugins bundled with it
//! and the person's own plugins. Discovery decides nothing: the registry lists
//! every structurally valid plugin. Whether one may run is decided once, when
//! a live guest starts, by [`PluginAdmission::start_live_plugin`]. It resolves
//! the decision against the profile's current grants and hands the start an
//! [`AdmittedPlugin`]: the only value the live Python and WASM constructors
//! accept, and the only source of the id, root and entry they run.
//!
//! - A profile install root holding a bundled id runs when its files match the
//!   copy built into this binary (the packager's file selection); otherwise it
//!   is refused with a reinstall hint. Other profile installs need one Allow
//!   in the Plexi window, keyed to the plugin id in this profile.
//! - Anything else runs when the person trusted the folder holding the plugin
//!   (a standalone component's own folder). An explicit path launch from a
//!   trusted folder may run a plugin with a bundled id, in its own pane only:
//!   discovery never lets a workspace own a bundled id (`reserved_id`).
//!
//! The allow is NOT content binding: later edits, reinstalls, `.pyc` files and
//! lazy imports run under it. Binding to the bytes a guest executes needs an
//! immutable execution tree; stints 1279 and 1293 own that.
//!
//! Every install source passes [`admit_install_source`]: bundled (`local:`)
//! sources and local folders install; git, marketplace and package-file
//! sources are refused before anything is fetched. No setting turns them on.

use std::path::{Path, PathBuf};

use crate::app::registry::{ManifestType, ResolvedPlugin};
use crate::broker::gate::{AdmitRequest, PermissionMonitor, PluginStartDecision};
use crate::broker::principal::Principal;
use crate::broker::{ActorScope, TargetType};
use crate::host::guest_text::shown;

/// Broker tool for loading one installed copy from the profile.
pub(crate) const LOAD_INSTALLED_TOOL: &str = "plugin.load";
/// Broker tool for trusting one folder of plugins.
pub(crate) const TRUST_FOLDER_TOOL: &str = "plugin.trust_folder";

/// Why a plugin was not admitted. Each message names what was refused, why,
/// and where to change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A git, marketplace or package-file install source.
    RemoteSource { source: String },
    /// A workspace or path tree that reuses the id of a bundled plugin.
    ShadowsBundled { id: String, dir: PathBuf },
    /// A local install that would replace a bundled plugin with other files.
    InstallOverBundled { id: String, dir: PathBuf },
    /// A profile install the person has not allowed.
    NotAllowed { id: String, dir: PathBuf },
    /// A plugin in a folder the person has not trusted.
    UntrustedFolder { id: String, folder: PathBuf },
    /// The person denied this plugin or folder and that denial is stored.
    Denied { id: String, dir: PathBuf },
    /// The path names no single plugin (the profile plugins folder itself).
    NotAPlugin {
        id: String,
        dir: PathBuf,
        error: String,
    },
    /// This process has no permission monitor, so it cannot read admissions.
    NotHost { id: String, dir: PathBuf },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemoteSource { source } => write!(
                f,
                "refused to install from '{}': this Plexi release installs only plugins \
                 bundled with it (local:<name>) and plugin folders on this computer \
                 (`plugin install ./path`). Git, marketplace and .plexipkg sources stay off \
                 until third-party installs open; no setting turns them on.",
                shown(source)
            ),
            Self::ShadowsBundled { id, dir } => write!(
                f,
                "plugin '{}' at {} was not loaded: it uses the id of a plugin bundled with \
                 Plexi, and a workspace or folder plugin cannot replace a bundled one. \
                 Change [plugin] id in its manifest.toml.",
                shown(id),
                dir.display()
            ),
            Self::InstallOverBundled { id, dir } => write!(
                f,
                "refused to install {} as '{}': that id belongs to a plugin bundled with Plexi \
                 and these files differ from it, so nothing was changed. Change [plugin] id in \
                 its manifest.toml, or develop it in place with `{} view open {}`.",
                dir.display(),
                shown(id),
                crate::cli::setup::cli_name(),
                dir.display()
            ),
            Self::NotAllowed { id, dir, .. } => write!(
                f,
                "plugin '{}' at {} was not loaded: it is not bundled with Plexi and has not \
                 been allowed. Allow it in the Plexi window (Cmd+I).",
                shown(id),
                dir.display()
            ),
            Self::UntrustedFolder { id, folder } => write!(
                f,
                "plugin '{}' was not loaded: plugins in {} are not trusted. Trust the folder \
                 in the Plexi window (Cmd+I).",
                shown(id),
                folder.display()
            ),
            Self::Denied { id, dir } => write!(
                f,
                "plugin '{}' at {} was not loaded: you denied it. Change that in the \
                 Permissions plugin or with `plexi permissions list`.",
                shown(id),
                dir.display()
            ),
            Self::NotAPlugin { id, dir, error } => write!(
                f,
                "plugin '{}' at {} was not loaded: {error}",
                shown(id),
                dir.display()
            ),
            Self::NotHost { id, dir } => write!(
                f,
                "plugin '{}' at {} was not loaded: only the Plexi host can check whether it \
                 was allowed",
                shown(id),
                dir.display()
            ),
        }
    }
}

/// Where an install would come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallSource<'a> {
    /// `local:<name>`, extracted from this binary.
    Bundled(&'a str),
    /// A plugin folder on this computer.
    Folder(&'a Path),
    /// `github:`, `git+...`, or a pack entry naming either.
    Git(&'a str),
    /// A bare id resolved through the marketplace.
    Marketplace(&'a str),
    /// A `.plexipkg` file.
    PackageFile(&'a Path),
}

/// The one decision on install sources. Callers refuse before any network
/// access or file write.
pub(crate) fn admit_install_source(source: InstallSource<'_>) -> Result<(), Refusal> {
    let refused = match source {
        InstallSource::Bundled(_) | InstallSource::Folder(_) => return Ok(()),
        InstallSource::Git(spec) | InstallSource::Marketplace(spec) => spec.to_string(),
        InstallSource::PackageFile(path) => path.display().to_string(),
    };
    let refusal = Refusal::RemoteSource { source: refused };
    log::info!("plugin_admission: install refused: {refusal}");
    Err(refusal)
}

/// Discovery never lets a workspace plugin own a bundled plugin's id, trusted
/// folder or not and identical bytes or not: the bundled entry always wins.
/// The CLI inventory applies this too.
pub(crate) fn reserved_id(id: &str, plugin_dir: &Path) -> Result<(), Refusal> {
    if crate::cli::install_host::is_bundled_plugin_id(id) {
        return Err(Refusal::ShadowsBundled {
            id: id.to_string(),
            dir: plugin_dir.to_path_buf(),
        });
    }
    Ok(())
}

/// A local folder may install under a bundled plugin's id only when it is that
/// plugin, byte for byte: anything else would replace a working bundled copy
/// with one that is refused at launch.
pub(crate) fn admit_local_install(id: &str, source: &Path) -> Result<(), Refusal> {
    if crate::cli::install_host::is_bundled_plugin_id(id)
        && !crate::cli::install_host::matches_bundled_plugin(id, source)
    {
        let refusal = Refusal::InstallOverBundled {
            id: id.to_string(),
            dir: source.to_path_buf(),
        };
        log::info!("plugin_admission: install refused: {refusal}");
        return Err(refusal);
    }
    log::info!(
        "plugin_admission: local install admitted: {} from {}",
        shown(id),
        source.display()
    );
    Ok(())
}

/// Why a live start was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Basis {
    /// A profile install root identical to the copy built into this binary.
    Bundled,
    /// The person allowed this plugin id in the profile.
    ProfileAllow,
    /// The person trusted the folder holding the plugin.
    TrustedFolder,
}

/// Permission to start one live guest, minted only by
/// [`PluginAdmission::start_live_plugin`] and consumed by the live Python and
/// WASM constructors, which take the plugin's id, root and entry only from it.
/// Private fields, no `Clone`, no deserialisation; its lifetime ends with the
/// synchronous start. While it lives the broker's admission lock is held for a
/// granted start, so a revoke cannot land between the decision and the start.
pub(crate) struct AdmittedPlugin<'start> {
    owner: String,
    kind: ManifestType,
    root: PathBuf,
    entry: PathBuf,
    basis: Basis,
    _commit: Option<std::sync::MutexGuard<'start, ()>>,
}

impl AdmittedPlugin<'_> {
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn kind(&self) -> ManifestType {
        self.kind
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn entry(&self) -> &Path {
        &self.entry
    }
    /// Why this start was admitted, for tests that assert the basis.
    #[cfg(test)]
    pub(crate) fn basis(&self) -> Basis {
        self.basis
    }

    /// Spend the permit at the start it was minted for. Every live guest start
    /// passes through here, so tests count starts by it. The returned value
    /// keeps the admission lock until the caller has started the guest.
    pub(crate) fn commit<'s>(self) -> CommittedStart<'s>
    where
        Self: 's,
    {
        log::info!(
            "plugin_admission: starting {} from {} ({:?})",
            shown(&self.owner),
            self.entry.display(),
            self.basis
        );
        #[cfg(test)]
        GUEST_STARTS.with(|starts| starts.set(starts.get() + 1));
        CommittedStart {
            owner: self.owner,
            root: self.root,
            entry: self.entry,
            _commit: self._commit,
        }
    }
}

/// What a spent permit names: the id, root and entry the guest runs. Drop it
/// once the guest has started; that ends the start's hold on the admission lock.
pub(crate) struct CommittedStart<'start> {
    pub owner: String,
    pub root: PathBuf,
    pub entry: PathBuf,
    _commit: Option<std::sync::MutexGuard<'start, ()>>,
}

#[cfg(test)]
thread_local! {
    static GUEST_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Live guest starts committed on this test thread.
#[cfg(test)]
pub(crate) fn guest_starts_for_test() -> usize {
    GUEST_STARTS.with(std::cell::Cell::get)
}

/// The grant subject a start needs.
enum Subject {
    /// A profile install root: the plugin id in this profile.
    Copy { id: String, dir: PathBuf },
    /// Any other tree: the folder holding it.
    Folder { id: String, folder: PathBuf },
}

impl Subject {
    fn principal(&self) -> Principal {
        match self {
            Self::Copy { id, .. } => Principal::plugin(id.clone(), None),
            Self::Folder { .. } => Principal::LocalUser,
        }
    }

    fn admit_request<'a>(
        &'a self,
        profile_plugins: &'a Path,
        principal: &'a Principal,
    ) -> AdmitRequest<'a> {
        let (tool, workspace_root) = match self {
            Self::Copy { .. } => (LOAD_INSTALLED_TOOL, profile_plugins),
            Self::Folder { folder, .. } => (TRUST_FOLDER_TOOL, folder.as_path()),
        };
        AdmitRequest {
            call_id: "",
            tool,
            input_json: "{}",
            principal,
            actor_scope: ActorScope::User,
            trust_origin: "host",
            workspace_root,
            context_id: 0,
            package_id: "",
            instance_id: 0,
            target_type: TargetType::Package,
        }
    }

    fn disclosure(&self) -> String {
        match self {
            Self::Copy { id, dir } => format!(
                "Load plugin {} from {}? It is not bundled with Plexi. Allow only a plugin you \
                 wrote or trust. Allowing covers this plugin id in this profile, including later \
                 edits or reinstalls of it.",
                shown(id),
                dir.display()
            ),
            Self::Folder { id, folder } => format!(
                "Trust plugins in {}? {} asked to load from there. Trusting the folder lets Plexi \
                 load any plugin in it, including later changes. Trust only a folder you control.",
                folder.display(),
                shown(id)
            ),
        }
    }

    fn waiting(&self) -> Refusal {
        match self {
            Self::Copy { id, dir } => Refusal::NotAllowed {
                id: id.clone(),
                dir: dir.clone(),
            },
            Self::Folder { id, folder } => Refusal::UntrustedFolder {
                id: id.clone(),
                folder: folder.clone(),
            },
        }
    }

    fn denied(&self) -> Refusal {
        let (id, dir) = match self {
            Self::Copy { id, dir } => (id, dir),
            Self::Folder { id, folder } => (id, folder),
        };
        Refusal::Denied {
            id: id.clone(),
            dir: dir.clone(),
        }
    }
}

/// Mint the permit for a decided start. Private to this module: no other code
/// can produce an [`AdmittedPlugin`].
fn mint<'start>(
    plugin: &ResolvedPlugin,
    basis: Basis,
    commit: Option<std::sync::MutexGuard<'start, ()>>,
) -> AdmittedPlugin<'start> {
    AdmittedPlugin {
        owner: plugin.owner.clone(),
        kind: plugin.kind,
        root: plugin.root.clone(),
        entry: plugin.entry.clone(),
        basis,
        _commit: commit,
    }
}

/// The admission check for one profile.
#[derive(Debug, Clone)]
pub(crate) struct PluginAdmission {
    profile_dir: PathBuf,
    profile_plugins: PathBuf,
}

impl PluginAdmission {
    /// `profile_dir` holds the grant store; `profile_plugins` is the folder
    /// whose entries are installs rather than trusted folders.
    pub(crate) fn new(profile_dir: &Path, profile_plugins: &Path) -> Self {
        Self {
            profile_dir: profile_dir.to_path_buf(),
            profile_plugins: profile_plugins.to_path_buf(),
        }
    }

    /// The running profile's admission.
    pub(crate) fn current() -> Self {
        Self::new(
            &crate::config::config_dir(),
            &crate::app::registry::plugins_dir(),
        )
    }

    /// The profile plugins folder, resolved now: it may not have existed
    /// when this check was built, and paths compare in canonical form.
    fn plugins_folder(&self) -> PathBuf {
        crate::platform::path::canonical_or_self(&self.profile_plugins)
    }

    /// The one live start operation. Decides `plugin` against the profile's
    /// current grants and, when it may run, calls `start` with the permit. A
    /// refusal a click can lift files (or reuses) the ask the Plexi window
    /// shows; the error names what was refused and where to change it.
    pub(crate) fn start_live_plugin<T>(
        &self,
        plugin: &ResolvedPlugin,
        start: impl FnOnce(AdmittedPlugin<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let subject = match self.subject_for(plugin) {
            Ok(Some(subject)) => subject,
            Ok(None) => return start(mint(plugin, Basis::Bundled, None)),
            Err(refusal) => {
                log::info!("plugin_admission: refused: {refusal}");
                return Err(refusal.to_string());
            }
        };
        let Ok(monitor) = PermissionMonitor::for_profile(&self.profile_dir) else {
            let refusal = Refusal::NotHost {
                id: plugin.owner.clone(),
                dir: plugin.root.clone(),
            };
            log::info!("plugin_admission: refused: {refusal}");
            return Err(refusal.to_string());
        };
        let principal = subject.principal();
        let plugins = self.plugins_folder();
        let disclosure = subject.disclosure();
        let basis = match subject {
            Subject::Copy { .. } => Basis::ProfileAllow,
            Subject::Folder { .. } => Basis::TrustedFolder,
        };
        let decision =
            monitor.begin_plugin_start(subject.admit_request(&plugins, &principal), &disclosure);
        match decision {
            PluginStartDecision::Allowed(guard) => start(mint(plugin, basis, Some(guard))),
            PluginStartDecision::Required { pending_request_id } => {
                let refusal = subject.waiting();
                log::info!("plugin_admission: refused, asked {pending_request_id}: {refusal}");
                Err(format!(
                    "{refusal} The request is waiting there ({pending_request_id})."
                ))
            }
            PluginStartDecision::Denied => {
                let refusal = subject.denied();
                log::info!("plugin_admission: refused: {refusal}");
                Err(refusal.to_string())
            }
        }
    }

    /// What `plugin` needs: nothing (`None`: a verified bundle), the profile
    /// Allow for its id (a profile install root), or trust for the folder
    /// holding it (anywhere else).
    fn subject_for(&self, plugin: &ResolvedPlugin) -> Result<Option<Subject>, Refusal> {
        let plugins = self.plugins_folder();
        if !plugin.root.starts_with(&plugins) {
            return Ok(Some(Subject::Folder {
                id: plugin.owner.clone(),
                folder: plugin.trust_folder(),
            }));
        }
        if plugin.standalone || plugin.root.parent() != Some(plugins.as_path()) {
            return Err(Refusal::NotAPlugin {
                id: plugin.owner.clone(),
                dir: plugin.root.clone(),
                error: format!(
                    "only a plugin installed directly in {} runs from the profile",
                    plugins.display()
                ),
            });
        }
        if crate::cli::install_host::is_bundled_plugin_id(&plugin.owner) {
            let at_its_root = plugin.root.file_name() == Some(std::ffi::OsStr::new(&plugin.owner));
            if at_its_root
                && crate::cli::install_host::matches_bundled_plugin(&plugin.owner, &plugin.root)
            {
                return Ok(None);
            }
            return Err(Refusal::NotAPlugin {
                id: plugin.owner.clone(),
                dir: plugin.root.clone(),
                error: format!(
                    "it uses the id of a plugin bundled with Plexi but differs from the copy \
                     shipped with this version; `{} plugin install local:{}` replaces it with \
                     the shipped copy",
                    crate::cli::setup::cli_name(),
                    plugin.owner
                ),
            });
        }
        Ok(Some(Subject::Copy {
            id: plugin.owner.clone(),
            dir: plugin.root.clone(),
        }))
    }
}

#[cfg(test)]
impl AdmittedPlugin<'static> {
    /// A permit for tests that build a live guest directly from a fixture,
    /// outside any route. Compiled out of every non-test build.
    pub(crate) fn for_test(owner: &str, kind: ManifestType, root: &Path, entry: &Path) -> Self {
        Self {
            owner: owner.to_string(),
            kind,
            root: root.to_path_buf(),
            entry: entry.to_path_buf(),
            basis: Basis::TrustedFolder,
            _commit: None,
        }
    }
}

#[cfg(test)]
impl PluginAdmission {
    /// Settle the plugin-code ask for `subject` with Allow, as a desktop click
    /// does. Nothing happens when it is already allowed.
    fn allow_for_test(&self, subject: Subject) {
        let monitor =
            PermissionMonitor::for_profile(&self.profile_dir).expect("tests run as the host");
        let principal = subject.principal();
        let plugins = self.plugins_folder();
        let decision = monitor.begin_plugin_start(
            subject.admit_request(&plugins, &principal),
            &subject.disclosure(),
        );
        match decision {
            PluginStartDecision::Allowed(_) => {}
            PluginStartDecision::Required { pending_request_id } => monitor
                .approve_pending(
                    &pending_request_id,
                    crate::broker::gate::ApprovalChoice::Always,
                )
                .expect("allow recorded"),
            PluginStartDecision::Denied => panic!("a stored denial blocks this test allow"),
        };
    }

    /// Allow profile plugin `id` the way a desktop click does.
    pub(crate) fn allow_copy_for_test(&self, id: &str, plugin_dir: &Path) {
        self.allow_for_test(Subject::Copy {
            id: id.to_string(),
            dir: crate::platform::path::canonical_or_self(plugin_dir),
        });
    }

    /// Trust `folder` the way a desktop click does. For tests that launch
    /// fixture plugins from a folder.
    pub(crate) fn trust_folder_for_test(&self, folder: &Path) {
        self.allow_for_test(Subject::Folder {
            id: "test-fixture".to_string(),
            folder: crate::platform::path::canonical_or_self(folder),
        });
    }
}

/// Trust `root`'s workspace plugin folder in the current test profile, as a
/// click does. For host tests whose fixtures are workspace plugins.
#[cfg(test)]
pub(crate) fn trust_workspace_plugins_for_test(root: &Path) {
    let plugins = crate::app::registry::workspace_plugins_dir(root);
    std::fs::create_dir_all(&plugins).expect("create workspace plugins folder");
    PluginAdmission::current().trust_folder_for_test(&plugins);
}

#[cfg(test)]
#[path = "plugin_admission_tests.rs"]
mod tests;
