//! Which plugin code the host loads, and which install sources are accepted
//! (stint 1398).
//!
//! Until third-party installs open, Plexi runs only plugins bundled with it
//! and the person's own plugins. Every plugin tree the host would load passes
//! [`PluginAdmission::decide`]:
//!
//! - In the profile plugins folder: admitted when it is bundled (byte-identical
//!   to the copy built into this binary) or when the person allowed this exact
//!   content in the Plexi window. A changed copy asks again.
//! - Anywhere else (a workspace's `<channel_dir>/plugins` or `agents`, a path
//!   launch): admitted when the person trusted the folder that holds it.
//!
//! Every install source passes [`admit_install_source`]: bundled (`local:`)
//! sources and local folders install; git, marketplace and package-file
//! sources are refused before anything is fetched. Nothing turns them on in
//! this release: a config key would let any process that can edit a file
//! change it, and the rule is that only a click allows code.
//!
//! The only way to allow is a desktop click. Admission is a sealed broker
//! grant (`TargetType::Package`) that only `PermissionMonitor::approve_pending`
//! mints; the registry scan reads it with `stored_decision` and never asks.

use std::path::{Path, PathBuf};

use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
use crate::broker::principal::Principal;
use crate::broker::{ActorScope, Decision, TargetType};
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
    /// A profile install the person has not allowed for this content.
    NotAllowed {
        id: String,
        dir: PathBuf,
        digest: String,
    },
    /// A plugin in a folder the person has not trusted.
    UntrustedFolder { id: String, folder: PathBuf },
    /// The person denied this plugin or folder and that denial is stored.
    Denied { id: String, dir: PathBuf },
    /// The plugin's content could not be read to identify it.
    Unreadable {
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
            Self::NotAllowed { id, dir, .. } => write!(
                f,
                "plugin '{}' at {} was not loaded: it is not bundled with Plexi and this copy \
                 has not been allowed. Allow it in the Plexi window (Cmd+I); a changed copy \
                 asks again.",
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
            Self::Unreadable { id, dir, error } => write!(
                f,
                "plugin '{}' at {} was not loaded: its files could not be read to identify \
                 it ({error})",
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

impl Refusal {
    /// True when a click in the Plexi window can admit the plugin.
    pub(crate) fn can_ask(&self) -> bool {
        matches!(self, Self::NotAllowed { .. } | Self::UntrustedFolder { .. })
    }
}

/// Why a plugin was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admitted {
    /// Identical to the copy built into this binary.
    Bundled,
    /// The person allowed this exact profile copy.
    AllowedCopy,
    /// The person trusted the folder that holds it.
    TrustedFolder,
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

/// A plugin outside the profile plugins folder may not take the id of a
/// plugin bundled with Plexi, trusted folder or not, unless it is an
/// identical copy of that plugin (a Plexi checkout's own `plugins/<id>`).
/// The CLI inventory applies this too, so no listing shows a workspace copy
/// in a bundled plugin's place.
pub(crate) fn reserved_id(id: &str, plugin_dir: &Path) -> Result<(), Refusal> {
    if crate::cli::install_host::is_bundled_plugin_id(id)
        && !crate::cli::install_host::matches_bundled_plugin(id, plugin_dir)
    {
        return Err(Refusal::ShadowsBundled {
            id: id.to_string(),
            dir: plugin_dir.to_path_buf(),
        });
    }
    Ok(())
}

/// The admission check for one profile. The host builds one per registry
/// view; the permission monitor is opened only when a non-bundled plugin is
/// decided.
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
            profile_plugins: crate::platform::path::canonical_or_self(profile_plugins),
        }
    }

    /// Decide whether the host may load plugin `id` from `plugin_dir`. Reads
    /// stored decisions only; never asks. Traces every refusal and every
    /// admitted non-bundled plugin at info.
    pub(crate) fn decide(&self, id: &str, plugin_dir: &Path) -> Result<Admitted, Refusal> {
        let verdict = self.decide_inner(id, plugin_dir);
        match &verdict {
            Ok(Admitted::Bundled) => {}
            Ok(how) => log::info!(
                "plugin_admission: admitted {} at {} ({how:?})",
                shown(id),
                plugin_dir.display()
            ),
            Err(refusal) => log::info!("plugin_admission: refused: {refusal}"),
        }
        verdict
    }

    fn decide_inner(&self, id: &str, plugin_dir: &Path) -> Result<Admitted, Refusal> {
        let Some(request) = self.request_for(id, plugin_dir)? else {
            return Ok(Admitted::Bundled);
        };
        let Ok(monitor) = PermissionMonitor::for_profile(&self.profile_dir) else {
            return Err(Refusal::NotHost {
                id: id.to_string(),
                dir: plugin_dir.to_path_buf(),
            });
        };
        let principal = request.principal();
        match monitor.stored_decision(&request.admit_request(&self.profile_plugins, &principal)) {
            Decision::Allow => Ok(match request {
                Request::Copy { .. } => Admitted::AllowedCopy,
                Request::Folder { .. } => Admitted::TrustedFolder,
            }),
            Decision::Deny => Err(Refusal::Denied {
                id: id.to_string(),
                dir: plugin_dir.to_path_buf(),
            }),
            Decision::Ask => Err(match request {
                Request::Copy { id, dir, digest } => Refusal::NotAllowed { id, dir, digest },
                Request::Folder { id, folder } => Refusal::UntrustedFolder { id, folder },
            }),
        }
    }

    /// What `plugin_dir` needs: nothing (`None`: bundled), an allowed copy
    /// (anything inside the profile plugins folder, decided for the install
    /// folder that holds it) or a trusted folder (anywhere else: the folder
    /// that holds `plugin_dir`).
    fn request_for(&self, id: &str, plugin_dir: &Path) -> Result<Option<Request>, Refusal> {
        let path = crate::platform::path::canonical_or_self(plugin_dir);
        let Ok(inside) = path.strip_prefix(&self.profile_plugins) else {
            if crate::cli::install_host::is_bundled_plugin_id(id) {
                // Either an identical bundled copy, which needs no click, or
                // a refusal no click can lift.
                return reserved_id(id, &path).map(|()| None);
            }
            let folder = path.parent().map(Path::to_path_buf).unwrap_or(path);
            return Ok(Some(Request::Folder {
                id: id.to_string(),
                folder,
            }));
        };
        let Some(install) = inside.components().next() else {
            return Err(Refusal::Unreadable {
                id: id.to_string(),
                dir: path.clone(),
                error: "the profile plugins folder itself is not a plugin".to_string(),
            });
        };
        let dir = self.profile_plugins.join(install);
        if crate::cli::install_host::is_bundled_plugin_id(id)
            && crate::cli::install_host::matches_bundled_plugin(id, &dir)
        {
            return Ok(None);
        }
        let digest =
            crate::app::package::content_digest(&dir).map_err(|error| Refusal::Unreadable {
                id: id.to_string(),
                dir: dir.clone(),
                error: error.to_string(),
            })?;
        Ok(Some(Request::Copy {
            id: id.to_string(),
            dir,
            digest,
        }))
    }

    /// Decide a launch, and when a click can lift the refusal, file the ask
    /// so the Plexi window shows it. `Err` is the message the launch fails with.
    pub(crate) fn admit_or_ask(&self, id: &str, plugin_dir: &Path) -> Result<Admitted, String> {
        self.decide(id, plugin_dir)
            .map_err(|refusal| self.refusal_message(&refusal))
    }

    /// The launch error for `refusal`, after filing its ask when it has one.
    pub(crate) fn refusal_message(&self, refusal: &Refusal) -> String {
        if !refusal.can_ask() {
            return refusal.to_string();
        }
        match self.ask(refusal) {
            Ok(pending) => format!("{refusal} The request is waiting there ({pending})."),
            Err(error) => {
                log::error!("plugin_admission: could not file the ask: {error}");
                format!("{refusal} The request could not be filed: {error}")
            }
        }
    }

    /// File the ask that admits what `refusal` refused, or reuse the one
    /// already waiting. Returns the pending request id the Plexi window
    /// shows. Only a desktop click on that ask mints the grant.
    pub(crate) fn ask(&self, refusal: &Refusal) -> Result<String, String> {
        let (request, disclosure) = match refusal {
            Refusal::NotAllowed { id, dir, digest } => (
                Request::Copy {
                    id: id.clone(),
                    dir: dir.clone(),
                    digest: digest.clone(),
                },
                format!(
                    "Load plugin {} from {}? It is not bundled with Plexi. Allow only a plugin \
                     you wrote or trust. Allowing covers this copy; a changed copy asks again.",
                    shown(id),
                    dir.display()
                ),
            ),
            Refusal::UntrustedFolder { id, folder } => (
                Request::Folder {
                    id: id.clone(),
                    folder: folder.clone(),
                },
                format!(
                    "Trust plugins in {}? {} asked to load from there. Trusting the folder lets \
                     Plexi load any plugin in it, including later changes. Trust only a folder \
                     you control.",
                    folder.display(),
                    shown(id)
                ),
            ),
            other => return Err(other.to_string()),
        };
        let monitor = PermissionMonitor::for_profile(&self.profile_dir)
            .map_err(|error| format!("plugin admission cannot ask: {error}"))?;
        let principal = request.principal();
        match monitor.admit_plugin_code(
            request.admit_request(&self.profile_plugins, &principal),
            &disclosure,
        ) {
            Admission::Required { pending_request_id } => {
                log::info!(
                    "plugin_admission: asked {pending_request_id} for {}",
                    request.label()
                );
                Ok(pending_request_id)
            }
            Admission::Proceed { .. } => Err(format!(
                "{} was allowed meanwhile; open it again",
                request.label()
            )),
            Admission::Denied { code } => Err(format!("{refusal} ({code})")),
        }
    }
}

/// The broker request behind one admission.
enum Request {
    Copy {
        id: String,
        dir: PathBuf,
        digest: String,
    },
    Folder {
        id: String,
        folder: PathBuf,
    },
}

impl Request {
    fn label(&self) -> String {
        match self {
            Self::Copy { id, .. } => format!("plugin {}", shown(id)),
            Self::Folder { folder, .. } => format!("plugin folder {}", folder.display()),
        }
    }

    /// Who the decision is recorded for: a copy is the plugin's own, bound
    /// to its content; a folder is the local user's.
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
        let (tool, workspace_root, package_id): (_, &Path, &str) = match self {
            Self::Copy { digest, .. } => (LOAD_INSTALLED_TOOL, profile_plugins, digest),
            Self::Folder { folder, .. } => (TRUST_FOLDER_TOOL, folder, ""),
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
            package_id,
            instance_id: 0,
            target_type: TargetType::Package,
        }
    }
}

#[cfg(test)]
impl PluginAdmission {
    /// Allow the profile copy at `plugin_dir` the way a desktop click does.
    /// For tests that install fixture plugins into a profile folder.
    pub(crate) fn allow_copy_for_test(&self, id: &str, plugin_dir: &Path) {
        let refusal = match self.decide(id, plugin_dir) {
            Ok(_) => return,
            Err(refusal @ Refusal::NotAllowed { .. }) => refusal,
            Err(other) => panic!("{id} is not a profile copy: {other}"),
        };
        let pending = self.ask(&refusal).expect("allow ask filed");
        PermissionMonitor::for_profile(&self.profile_dir)
            .expect("tests run as the host")
            .approve_pending(&pending, crate::broker::gate::ApprovalChoice::Always)
            .expect("allow recorded");
    }

    /// Trust `folder` the way a desktop click does: file the ask and settle it
    /// with Allow. For tests that launch fixture plugins from a folder.
    pub(crate) fn trust_folder_for_test(&self, folder: &Path) {
        let refusal = Refusal::UntrustedFolder {
            id: "test-fixture".to_string(),
            folder: crate::platform::path::canonical_or_self(folder),
        };
        let monitor =
            PermissionMonitor::for_profile(&self.profile_dir).expect("tests run as the host");
        let principal = Principal::LocalUser;
        let request = Request::Folder {
            id: "test-fixture".to_string(),
            folder: crate::platform::path::canonical_or_self(folder),
        };
        if monitor.stored_decision(&request.admit_request(&self.profile_plugins, &principal))
            == Decision::Allow
        {
            return;
        }
        let pending = self.ask(&refusal).expect("trust ask filed");
        monitor
            .approve_pending(&pending, crate::broker::gate::ApprovalChoice::Always)
            .expect("trust recorded");
    }
}

/// Trust `root`'s workspace plugin folder in the current test profile, as a
/// click does. For host tests whose fixtures are workspace plugins.
#[cfg(test)]
pub(crate) fn trust_workspace_plugins_for_test(root: &Path) {
    let plugins = crate::app::registry::workspace_plugins_dir(root);
    std::fs::create_dir_all(&plugins).expect("create workspace plugins folder");
    PluginAdmission::new(
        &crate::config::config_dir(),
        &crate::app::registry::plugins_dir(),
    )
    .trust_folder_for_test(&plugins);
}

#[cfg(test)]
#[path = "plugin_admission_tests.rs"]
mod tests;
