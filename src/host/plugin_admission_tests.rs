//! Stint 1398: a live guest starts only for a bundled plugin, an allowed
//! profile plugin or a trusted folder, decided at the start; discovery lists
//! every valid plugin and asks nothing; remote install sources are refused
//! before any fetch.

use super::*;
use crate::app::registry::{PluginRegistry, RegistrySource};
use crate::broker::gate::ApprovalChoice;
use crate::cli::install_host::test_support::{extract_bundled, MockCloner};

fn write_plugin(parent: &Path, id: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("main.py"), "print('hi')\n").unwrap();
    dir
}

/// A profile (grant store) with its plugins folder, and a cloned repo that is
/// a workspace for this channel.
struct Rig {
    profile: tempfile::TempDir,
    repo: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let rig = Self {
            profile: tempfile::tempdir().unwrap(),
            repo: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(rig.plugins()).unwrap();
        std::fs::create_dir_all(rig.repo_plugins()).unwrap();
        rig
    }
    fn plugins(&self) -> PathBuf {
        self.profile.path().join("plugins")
    }
    fn repo_plugins(&self) -> PathBuf {
        self.repo
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("plugins")
    }
    fn admission(&self) -> PluginAdmission {
        PluginAdmission::new(self.profile.path(), &self.plugins())
    }
    /// The discovery inventory: metadata only, it decides nothing.
    fn load(&self) -> PluginRegistry {
        PluginRegistry::load_with_global(self.repo.path(), &self.plugins())
    }
    fn monitor(&self) -> std::sync::Arc<PermissionMonitor> {
        PermissionMonitor::for_profile(self.profile.path()).unwrap()
    }
    /// Start the plugin at `dir` through the one start operation; on success
    /// return why it was admitted.
    fn start(&self, dir: &Path) -> Result<Basis, String> {
        let resolved = crate::app::registry::resolve_plugin_dir(dir)?;
        self.admission()
            .start_live_plugin(&resolved, |admitted| Ok(admitted.basis()))
    }
    /// The plugin-code ask waiting in the Plexi window, if one is.
    fn pending_ask(&self) -> Option<String> {
        self.monitor()
            .list_pending()
            .into_iter()
            .find(|pending| {
                pending.tool == LOAD_INSTALLED_TOOL || pending.tool == TRUST_FOLDER_TOOL
            })
            .map(|pending| pending.pending_request_id)
    }
    /// What a desktop click does: settle the waiting ask.
    fn click(&self, choice: ApprovalChoice) -> Result<(), String> {
        let pending = self.pending_ask().expect("an ask is waiting");
        self.monitor().approve_pending(&pending, choice)
    }
}

/// (a) A cloned repo's `<channel_dir>/plugins/<bundled id>` took the bundled
/// plugin's place with no install step. Discovery keeps the bundled owner.
#[test]
fn workspace_plugin_with_a_bundled_id_never_shadows_the_bundled_one() {
    let rig = Rig::new();
    extract_bundled("todo", &rig.plugins());
    write_plugin(&rig.repo_plugins(), "todo");
    let registry = rig.load();
    let todo = registry.get("todo").expect("bundled todo listed");
    assert_eq!(todo.source, RegistrySource::Global, "bundled copy wins");
    let skipped = registry
        .skipped_reason("todo")
        .expect("workspace copy skipped");
    assert!(
        skipped.error.contains("Change [plugin] id"),
        "{}",
        skipped.error
    );
}

/// A cloned repo's plugin is discovered with zero asks; starting it asks for
/// its folder; Deny refuses; Allow starts it; an edit needs no new click.
#[test]
fn cloned_repo_plugin_starts_only_after_its_folder_is_trusted() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.repo_plugins(), "repo-tool");
    assert!(rig.load().get("repo-tool").is_some(), "discovered");
    assert!(rig.pending_ask().is_none(), "a scan asks nothing");

    let error = rig.start(&dir).unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert!(error.contains("Cmd+I"), "{error}");
    rig.click(ApprovalChoice::Deny).unwrap();
    assert!(rig.start(&dir).is_err(), "deny admits nothing");

    rig.click(ApprovalChoice::Always).unwrap();
    assert_eq!(rig.start(&dir), Ok(Basis::TrustedFolder));
    std::fs::write(dir.join("main.py"), "print('edited')\n").unwrap();
    assert_eq!(
        rig.start(&dir),
        Ok(Basis::TrustedFolder),
        "trust covers edits"
    );
}

/// The CLI cannot bypass the host: a tree written straight into the profile
/// plugins folder (as a git clone would) does not start until its id is
/// allowed by a click. The allow is keyed to the id, not content (stint 1279).
#[test]
fn planted_profile_plugin_starts_only_after_it_is_allowed() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.plugins(), "cloned");
    let error = rig.start(&dir).unwrap_err();
    assert!(error.contains("has not been allowed"), "{error}");
    assert!(
        rig.click(ApprovalChoice::Once).is_err(),
        "a plugin load is allowed or denied, never once"
    );
    rig.click(ApprovalChoice::Always).unwrap();
    assert_eq!(rig.start(&dir), Ok(Basis::ProfileAllow));
    PermissionMonitor::reload_for_test(rig.profile.path());
    assert_eq!(
        rig.start(&dir),
        Ok(Basis::ProfileAllow),
        "the allow is sealed on disk and survives a restart"
    );
}

/// A bundled plugin at its install root starts with no click. A profile copy
/// of a bundled id whose files differ from the shipped copy is refused with a
/// reinstall hint: no click makes a changed copy the bundled plugin.
#[test]
fn a_bundled_id_in_the_profile_runs_only_as_the_shipped_copy() {
    let rig = Rig::new();
    let todo = extract_bundled("todo", &rig.plugins());
    assert_eq!(rig.start(&todo), Ok(Basis::Bundled));
    assert!(rig.pending_ask().is_none(), "no ask for bundled");

    std::fs::write(todo.join("main.py"), "print('not bundled')\n").unwrap();
    let error = rig.start(&todo).unwrap_err();
    assert!(error.contains("plugin install local:todo"), "{error}");
    assert!(rig.pending_ask().is_none(), "no click can lift it");
}

/// Only a plugin installed directly in the profile plugins folder runs from
/// the profile; a manifest nested inside an install is neither an install nor
/// a trustable folder.
#[test]
fn a_plugin_nested_inside_a_profile_install_is_refused() {
    let rig = Rig::new();
    let outer = rig.plugins().join("outer");
    let inner = write_plugin(&outer, "inner");
    let error = rig.start(&inner).unwrap_err();
    assert!(error.contains("installed directly in"), "{error}");
    assert!(rig.pending_ask().is_none());
}

/// (b) `plugin install github:...` was admitted. The decision refuses every
/// remote source; bundled sources and local folders pass.
#[test]
fn remote_install_sources_are_refused() {
    for spec in [
        "github:someone/plugin",
        "git+https://example.com/plugin.git",
        "some-marketplace-id",
    ] {
        let refusal = crate::cli::install::admit_install_spec(spec).unwrap_err();
        let text = refusal.to_string();
        assert!(text.contains(spec), "names what was refused: {text}");
        assert!(text.contains("no setting turns them on"), "{text}");
    }
    assert!(crate::cli::install::admit_install_spec("local:todo").is_ok());
    assert!(admit_install_source(InstallSource::Folder(Path::new("/tmp/mine"))).is_ok());
    assert!(admit_install_source(InstallSource::PackageFile(Path::new("x.plexipkg"))).is_err());
}

/// (b) A pack naming a git source installs nothing and clones nothing: the
/// refusal sits at the shared install boundary every caller goes through.
#[test]
fn workspace_pack_with_a_git_entry_clones_nothing() {
    let rig = Rig::new();
    let channel = rig.repo.path().join(crate::config::workspace_channel_dir());
    std::fs::write(
        channel.join("plugins.toml"),
        "schema_version = 2\n\n[[plugin]]\nid = \"todo\"\nsource = \"local:todo\"\nversion = \"bundled\"\n\n[[plugin]]\nid = \"remote\"\nsource = \"github:someone/remote\"\nversion = \"v1\"\n",
    )
    .unwrap();
    let cloner = MockCloner::new();
    let outcomes = crate::cli::install_host::apply_workspace_pack(rig.repo.path(), &cloner)
        .expect("the pack is read");
    for outcome in &outcomes {
        let crate::cli::install_host::InstallStatus::Failed(error) = &outcome.status else {
            panic!("{} must be refused: {:?}", outcome.id, outcome.status);
        };
        let expected = if outcome.id == "todo" {
            "bundled with Plexi"
        } else {
            "github:someone/remote"
        };
        assert!(error.contains(expected), "{}: {error}", outcome.id);
    }
    assert_eq!(outcomes.len(), 2);
    assert!(cloner.clones.lock().unwrap().is_empty(), "nothing fetched");
    assert!(
        !rig.repo_plugins().join("todo").exists(),
        "the pack is refused whole, not half-applied"
    );
}

/// The host itself refuses: a path launch from an untrusted folder and an id
/// launch of an untrusted workspace plugin both fail with the refusal and file
/// the ask the Plexi window shows. Nothing is approved here.
#[test]
fn host_launches_from_untrusted_folders_are_refused_and_ask() {
    let mut h = crate::testing::HostHarness::new();
    let fixtures = tempfile::tempdir().unwrap();
    let dir = write_plugin(fixtures.path(), "outside");
    let error = h
        .app
        .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, None, &[])
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert!(error.contains("waiting there"), "{error}");

    let repo_plugins = h
        .workspace_root()
        .join(crate::config::workspace_channel_dir())
        .join("plugins");
    write_plugin(&repo_plugins, "repo-tool");
    let error = h
        .app
        .launch_view_by_id_with_layout("repo-tool", None, &[], None)
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");

    let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
    let asks: Vec<_> = monitor
        .list_pending()
        .into_iter()
        .filter(|pending| pending.tool == TRUST_FOLDER_TOOL)
        .collect();
    assert_eq!(asks.len(), 2, "one ask per folder: {asks:?}");
}

/// Remove every `__pycache__` folder and `.pyc` under `dir`, as the packager's
/// ignore patterns do when it seeds an alpha profile.
fn strip_bytecode(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                std::fs::remove_dir_all(&path).unwrap();
            } else {
                strip_bytecode(&path);
            }
        } else if path.extension().is_some_and(|ext| ext == "pyc") {
            std::fs::remove_file(&path).unwrap();
        }
    }
}

/// Stint 1497: an unsigned update rotates the seal key and resets grants.
/// Every plugin allow and folder trust goes with them, and the plugins are
/// refused and asked about again; nothing starts silently.
#[test]
fn a_seal_key_reset_drops_admissions_and_fails_closed() {
    let rig = Rig::new();
    let mine = write_plugin(&rig.plugins(), "mine");
    let repo_tool = write_plugin(&rig.repo_plugins(), "repo-tool");
    rig.admission().allow_copy_for_test("mine", &mine);
    rig.admission().trust_folder_for_test(&rig.repo_plugins());
    assert!(rig.start(&mine).is_ok() && rig.start(&repo_tool).is_ok());

    crate::broker::seal::reset_unverified_profile(
        &crate::broker::custody::HostProcess::for_test(),
        rig.profile.path(),
    )
    .unwrap();
    PermissionMonitor::reload_for_test(rig.profile.path());

    assert!(rig
        .start(&mine)
        .unwrap_err()
        .contains("has not been allowed"));
    assert!(rig
        .start(&repo_tool)
        .unwrap_err()
        .contains("are not trusted"));
}

/// (a) The shared install boundary refuses a git source before the cloner
/// runs, whoever calls it.
#[test]
fn install_one_refuses_a_git_source_before_cloning() {
    let rig = Rig::new();
    let cloner = MockCloner::new();
    let error = crate::cli::install_host::install_one(
        &cloner,
        &crate::app::packs::SourceSpec::Git("https://github.com/someone/plugin.git".into()),
        None,
        &rig.plugins(),
    )
    .expect_err("git source refused");
    assert!(error.contains("someone/plugin"), "{error}");
    assert!(cloner.clones.lock().unwrap().is_empty(), "nothing fetched");
}

/// (g) A manifest whose entry resolves outside its plugin folder (an absolute
/// path, `..`, or a symlink out) is refused: the folder that was trusted or
/// allowed is not where the code would come from.
#[test]
fn an_entry_outside_the_plugin_folder_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("foreign.py");
    std::fs::write(&foreign, "print('outside')\n").unwrap();
    let parent = tempfile::tempdir().unwrap();
    let cases = [
        ("absolute", foreign.display().to_string()),
        ("parent", "../foreign.py".to_string()),
        ("symlink", "linked.py".to_string()),
    ];
    for (name, entry) in cases {
        let dir = parent.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[plugin]\nid = \"{name}\"\nname = \"{name}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"{entry}\"\n\n[[contributes.views]]\nid = \"{name}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(parent.path().join("foreign.py"), "print('outside')\n").unwrap();
        if name == "symlink" {
            std::os::unix::fs::symlink(&foreign, dir.join("linked.py")).unwrap();
        }
        let error = PluginRegistry::empty()
            .load_plugin(&dir)
            .expect_err("an entry outside the plugin folder must be refused");
        assert!(error.contains("outside"), "{name}: {error}");
    }
}

/// (c) Even an identical copy of a bundled plugin in a trusted workspace
/// folder does not own the id: discovery keeps the profile's bundled entry.
#[test]
fn an_identical_workspace_copy_never_owns_a_bundled_id() {
    let rig = Rig::new();
    extract_bundled("todo", &rig.plugins());
    extract_bundled("todo", &rig.repo_plugins());
    rig.admission().trust_folder_for_test(&rig.repo_plugins());
    let registry = rig.load();
    let todo = registry.get("todo").expect("bundled todo");
    assert_eq!(todo.source, RegistrySource::Global);
    assert_eq!(
        crate::platform::path::canonical_or_self(&todo.plugin_dir),
        crate::platform::path::canonical_or_self(&rig.plugins().join("todo"))
    );
    assert!(registry.skipped_reason("todo").is_some());
}

/// (e) Bundled-plugin development: a tree with a bundled id outside the
/// profile starts from a trusted folder (an explicit path launch); discovery of
/// that same tree is refused, and an untrusted folder still asks.
#[test]
fn an_explicit_path_launch_of_a_bundled_id_needs_only_folder_trust() {
    let rig = Rig::new();
    let dev = tempfile::tempdir().unwrap();
    let checkout = write_plugin(dev.path(), "todo");
    assert!(rig
        .start(&checkout)
        .unwrap_err()
        .contains("are not trusted"));
    rig.admission().trust_folder_for_test(dev.path());
    assert_eq!(rig.start(&checkout), Ok(Basis::TrustedFolder));
    assert!(
        reserved_id("todo", &checkout).is_err(),
        "discovery refuses it"
    );
}

/// The plugin tree a Python pane runs, canonical.
fn pane_tree(h: &crate::testing::HostHarness, pane: crate::spatial::tiling::PaneId) -> PathBuf {
    h.app
        .windows
        .iter()
        .find_map(|window| window.panes.get(&pane))
        .and_then(|pane| pane.as_view())
        .and_then(|view| match &view.runtime {
            crate::host::pane::ViewRuntime::Python(runtime) => Some(
                crate::platform::path::canonical_or_self(runtime.origin().root()),
            ),
            _ => None,
        })
        .expect("a Python pane")
}

/// (e) A dev copy of a bundled plugin path-launched from a trusted folder runs
/// in its own pane only. Opening the plugin by id (Cmd+P) still launches and
/// then focuses the bundled copy, never the dev pane, and the dev launch does
/// not focus the bundled pane either (chess is focus_existing_in_context).
#[test]
fn a_dev_copy_never_stands_in_for_the_bundled_plugin_opened_by_id() {
    let mut h = crate::testing::HostHarness::new();
    let profile_plugins = h.profile_dir().join("plugins");
    let bundled = extract_bundled("chess", &profile_plugins);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&profile_plugins);
    let dev = tempfile::tempdir().unwrap();
    let dev_chess = extract_bundled("chess", dev.path());
    let source = dev_chess.join("chess.py");
    let mut text = std::fs::read_to_string(&source).unwrap();
    text.push_str("\n# dev edit\n");
    std::fs::write(&source, text).unwrap();
    h.trust_plugin_folder(dev.path());
    let workspace = h.workspace_root();

    let open = |h: &mut crate::testing::HostHarness| h.state().open_panes;
    let before = open(&mut h);
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace.clone()))
        .expect("bundled chess by id");
    let bundled_pane = *open(&mut h)
        .iter()
        .find(|id| !before.contains(id))
        .expect("bundled pane");
    assert_eq!(
        pane_tree(&h, bundled_pane),
        crate::platform::path::canonical_or_self(&bundled)
    );

    let before = open(&mut h);
    h.app
        .launch_view_by_path_with_layout(
            &dev_chess.to_string_lossy(),
            None,
            Some(workspace.clone()),
            &[],
        )
        .expect("dev chess by path from a trusted folder");
    let dev_pane = *open(&mut h)
        .iter()
        .find(|id| !before.contains(id))
        .expect("the dev launch opens its own pane, not the bundled one");
    assert_eq!(
        pane_tree(&h, dev_pane),
        crate::platform::path::canonical_or_self(&dev_chess)
    );

    let before = open(&mut h);
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace))
        .expect("chess by id again");
    assert_eq!(open(&mut h), before, "by id focuses an existing pane");
    let focused = h.app.windows[h.app.active_window]
        .focused_pane
        .and_then(|tile| {
            h.app.windows[h.app.active_window]
                .tree
                .tiles
                .get_pane(&tile)
                .copied()
        })
        .expect("a focused pane");
    assert_eq!(
        focused, bundled_pane,
        "by id focuses the bundled pane, never the dev copy"
    );
    assert!(
        h.app
            .registries
            .view_for_root(&h.workspace_root())
            .get("chess")
            .is_some_and(|entry| {
                crate::platform::path::canonical_or_self(&entry.plugin_dir)
                    == crate::platform::path::canonical_or_self(&bundled)
            }),
        "the dev copy never enters the registry"
    );
}

/// (c) A workspace directory link that resolves to the bundled install itself
/// still never owns the id: the reservation applies to the scan source, before
/// the link's destination is classified.
#[test]
fn a_workspace_link_to_the_bundled_install_never_owns_its_id() {
    let rig = Rig::new();
    let bundled = extract_bundled("todo", &rig.plugins());
    std::os::unix::fs::symlink(&bundled, rig.repo_plugins().join("todo")).unwrap();
    rig.admission().trust_folder_for_test(&rig.repo_plugins());
    let registry = rig.load();
    let todo = registry.get("todo").expect("bundled todo");
    assert_eq!(todo.source, RegistrySource::Global);
    assert!(todo.workspace_root.is_none());
    assert!(registry.skipped_reason("todo").is_some());
}

/// (g) Python restore resolves the entry with the same resolver as discovery:
/// an absolute, parent-relative or symlinked entry outside the folder fails the
/// restore before any runtime is built.
#[test]
fn python_restore_refuses_an_entry_outside_the_plugin_folder() {
    let h = crate::testing::HostHarness::new();
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("foreign.py");
    std::fs::write(&foreign, "print('outside')\n").unwrap();
    let parent = tempfile::tempdir().unwrap();
    std::fs::write(parent.path().join("foreign.py"), "print('outside')\n").unwrap();
    PluginAdmission::current().trust_folder_for_test(parent.path());
    let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
    for (name, entry) in [
        ("absolute", foreign.display().to_string()),
        ("parent", "../foreign.py".to_string()),
        ("symlink", "linked.py".to_string()),
    ] {
        let dir = parent.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[plugin]\nid = \"{name}\"\nname = \"{name}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"{entry}\"\n\n[[contributes.views]]\nid = \"{name}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        if name == "symlink" {
            std::os::unix::fs::symlink(&foreign, dir.join("linked.py")).unwrap();
        }
        let workspace = h.workspace_root();
        let restored = crate::pane_ops::create::restore_view_pane_in_profile(
            name,
            &crate::workspace::SavedViewLaunch::Python {
                plugin_dir: dir.clone(),
                args: Vec::new(),
            },
            crate::pane_ops::create::RestoreTarget {
                pane_id: 4242,
                context_id: h.app.windows[0].context_id,
                workspace_root: workspace.clone(),
                context_root: &workspace,
                saved_name: None,
            },
            &colors,
            h.profile_dir(),
        );
        let error = restored.err().expect("restore must refuse");
        assert!(
            error.contains("outside the plugin folder"),
            "{name}: {error}"
        );
    }
}

/// (h) Finder metadata and a venv are not plugin files on either side of the
/// bundled comparison: a stray `.DS_Store` (root or nested) in an installed
/// bundled plugin does not make it ask.
#[test]
fn stray_finder_metadata_does_not_unbundle_a_plugin() {
    let rig = Rig::new();
    let todo = extract_bundled("todo", &rig.plugins());
    std::fs::write(todo.join(".DS_Store"), b"finder").unwrap();
    let nested = todo.join("tests");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join(".DS_Store"), b"finder").unwrap();
    std::fs::create_dir_all(nested.join(".venv/bin")).unwrap();
    assert_eq!(rig.start(&todo), Ok(Basis::Bundled));
    assert!(rig.pending_ask().is_none());
}

/// (h) The comparison skips exactly what `scripts/package.py` leaves out, so a
/// packaged copy and the embedded copy are compared on the same files.
#[test]
fn the_bundled_comparison_uses_the_packager_file_selection() {
    let script =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/package.py"))
            .unwrap();
    let line = script
        .lines()
        .find(|line| line.contains("shutil.ignore_patterns("))
        .expect("package.py ignore patterns");
    let args = &line[line.find("ignore_patterns(").unwrap() + "ignore_patterns(".len()..];
    let args = &args[..args.rfind(')').unwrap()];
    let packager: Vec<&str> = args
        .split(',')
        .map(|item| item.trim().trim_matches('"'))
        .collect();
    assert_eq!(packager, crate::cli::install_host::PACKAGER_EXCLUDES);
}

/// Bytecode is code, so it is not treated like Finder metadata: a planted
/// `.pyc` the binary does not embed means the copy is not the shipped one.
/// The Permissions bridge depends on this (`permission_ui_trust`).
#[test]
fn a_planted_pyc_in_a_bundled_plugin_is_not_the_shipped_copy() {
    let rig = Rig::new();
    let todo = extract_bundled("todo", &rig.plugins());
    let cache = todo.join("__pycache__");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("planted.cpython-312.pyc"), b"not bundled").unwrap();
    assert!(rig
        .start(&todo)
        .unwrap_err()
        .contains("differs from the copy"));
}

/// Copy `src` to `dst` as `scripts/package.py` does (its ignore patterns).
fn copy_like_packager(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "__pycache__"
            || name_str == ".venv"
            || name_str == ".DS_Store"
            || name_str.ends_with(".pyc")
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_like_packager(&path, &dst.join(&name));
        } else {
            std::fs::copy(&path, dst.join(&name)).unwrap();
        }
    }
}

/// Seed a harness profile with every maintained plugin using `seed`, then open
/// each one by id as Cmd+P does. Every open must start that plugin's own pane
/// with no admission ask (a plugin whose config is missing opens its
/// launch-failed pane, which is past admission).
fn open_every_maintained_plugin(seed: fn(&str, &Path)) {
    let mut h = crate::testing::HostHarness::new();
    let plugins = h.profile_dir().join("plugins");
    let repo_plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&repo_plugins).unwrap().flatten() {
        if entry.path().join("manifest.toml").is_file() {
            let id = entry.file_name().to_string_lossy().into_owned();
            seed(&id, &plugins);
            ids.push(id);
        }
    }
    assert!(ids.len() >= 10, "maintained plugins: {ids:?}");
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    let workspace = h.workspace_root();
    let mut started = 0;
    for id in &ids {
        let before = h.state().open_panes;
        let starts = guest_starts_for_test();
        h.app
            .launch_view_by_id_with_layout_forced(id, None, &[], Some(workspace.clone()))
            .unwrap_or_else(|error| panic!("{id} did not open on a fresh profile: {error}"));
        // A plugin whose config is missing opens its launch-failed pane and
        // starts no guest; every other one starts exactly one guest. An
        // overlay placement replaces the focused pane, so look for its pane.
        let guests = guest_starts_for_test() - starts;
        if guests == 1 {
            started += 1;
            assert!(
                h.app
                    .locate_view_instance(id, None, crate::pane_ops::create::OriginQuery::Any)
                    .is_some(),
                "{id} has a pane"
            );
        } else {
            assert_eq!(guests, 0, "{id}");
            assert!(
                h.state()
                    .open_panes
                    .iter()
                    .any(|pane| !before.contains(pane)),
                "{id} opened neither a guest nor a launch-failed pane"
            );
        }
    }
    assert_eq!(started, ids.len(), "every maintained guest starts: {ids:?}");
    let asks: Vec<_> = PermissionMonitor::for_profile(h.profile_dir())
        .unwrap()
        .list_pending()
        .into_iter()
        .filter(|pending| pending.tool == LOAD_INSTALLED_TOOL || pending.tool == TRUST_FOLDER_TOOL)
        .collect();
    assert!(asks.is_empty(), "a fresh profile asks: {asks:?}");
}

/// (h) A profile seeded from the checkout the way the packager does opens
/// every maintained plugin without asking, whatever `__pycache__` the build
/// checkout held.
#[test]
fn a_packaged_profile_opens_every_maintained_plugin_without_asking() {
    open_every_maintained_plugin(|id, plugins| {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins")
            .join(id);
        copy_like_packager(&src, &plugins.join(id));
    });
}

/// (h) A profile seeded from the binary's own embedded copies with bytecode
/// stripped (a binary built from a checkout holding `__pycache__`) opens every
/// maintained plugin without asking.
#[test]
fn an_embedded_seed_without_bytecode_opens_every_maintained_plugin_without_asking() {
    open_every_maintained_plugin(|id, plugins| strip_bytecode(&extract_bundled(id, plugins)));
}

/// A workspace pack naming a bundled plugin (`local:<name>`) would install a
/// copy discovery never lets the workspace own. The conflict is reported at
/// install time and nothing is written.
#[test]
fn a_workspace_pack_naming_a_bundled_plugin_reports_the_conflict() {
    let rig = Rig::new();
    let channel = rig.repo.path().join(crate::config::workspace_channel_dir());
    std::fs::write(
        channel.join("plugins.toml"),
        "schema_version = 2\n\n[[plugin]]\nid = \"todo\"\nsource = \"local:todo\"\nversion = \"bundled\"\n",
    )
    .unwrap();
    let outcomes =
        crate::cli::install_host::apply_workspace_pack(rig.repo.path(), &MockCloner::new())
            .expect("the pack is read");
    let [outcome] = outcomes.as_slice() else {
        panic!("{outcomes:?}");
    };
    let crate::cli::install_host::InstallStatus::Failed(error) = &outcome.status else {
        panic!("todo must be reported, not installed: {:?}", outcome.status);
    };
    assert!(error.contains("bundled with Plexi"), "{error}");
    assert!(!rig.repo_plugins().join("todo").exists(), "nothing written");
}

/// A WASM plugin folder named `id` whose entry is a copy of the timer fixture.
fn write_wasm_plugin(parent: &Path, id: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/timer-only.wasm"),
        dir.join("component.wasm"),
    )
    .unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[plugin.capabilities]\ncapabilities = [\"timer\"]\n\n[runtime]\nkind = \"wasm\"\nentry = \"component.wasm\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        ),
    )
    .unwrap();
    dir
}

/// Step 1: a saved layout's id is an assertion. Profile plugin `alpha` holds an
/// Allow; a saved pane names `alpha` but points at the component plugin `beta`
/// owns. The restore fails on identity before any grant, state or import under
/// `alpha` is touched.
#[test]
fn restore_with_a_saved_id_that_does_not_own_the_component_is_refused() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let h = crate::testing::HostHarness::new();
    let profile = h.profile_dir().to_path_buf();
    let admission = PluginAdmission::new(&profile, &profile.join("plugins"));
    let alpha = write_wasm_plugin(&profile.join("plugins"), "alpha");
    admission.allow_copy_for_test("alpha", &alpha);
    let elsewhere = tempfile::tempdir().unwrap();
    let beta = write_wasm_plugin(elsewhere.path(), "beta");
    let monitor = PermissionMonitor::for_profile(&profile).unwrap();
    let grants_before = monitor.list_entries().len();
    let workspace = h.workspace_root();
    let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
    let restored = crate::pane_ops::create::restore_view_pane_in_profile(
        "alpha",
        &crate::workspace::SavedViewLaunch::Wasm {
            wasm_path: beta.join("component.wasm"),
            args: Vec::new(),
        },
        crate::pane_ops::create::RestoreTarget {
            pane_id: 4343,
            context_id: h.app.windows[0].context_id,
            workspace_root: workspace.clone(),
            context_root: &workspace,
            saved_name: None,
        },
        &colors,
        &profile,
    );
    let error = restored.err().expect("identity mismatch refuses");
    assert!(error.contains("belongs to 'beta'"), "{error}");
    assert_eq!(
        monitor.list_entries().len(),
        grants_before,
        "no grant touched"
    );
    assert!(
        !profile.join("wasm-state").join("alpha").exists(),
        "no state opened under alpha"
    );
}

/// Step 1: a Python plugin whose entry is nested runs that exact file under
/// /app, not a same-named file at the plugin root.
#[test]
fn a_nested_python_entry_runs_the_exact_resolved_file() {
    let mut h = crate::testing::HostHarness::new();
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("nested-entry");
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        "schema_version = 2\n\n[plugin]\nid = \"nested-entry\"\nname = \"nested-entry\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"pkg/main.py\"\n\n[[contributes.views]]\nid = \"nested-entry.main\"\ndefault = true\n",
    )
    .unwrap();
    let body = |title: &str| {
        format!(
            "from plexi_sdk.effects import SetTitle\nfrom plexi_sdk.ui import Column\n\n\
             def init(size, args):\n    return [SetTitle({title:?})]\n\n\
             def update(event):\n    return []\n\n\
             def view():\n    return Column([])\n"
        )
    };
    std::fs::write(dir.join("pkg/main.py"), body("nested")).unwrap();
    std::fs::write(dir.join("main.py"), body("top")).unwrap();
    h.app.trust_plugin_path_for_test(&dir.to_string_lossy());
    let pane = h
        .app
        .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, None, &[])
        .expect("launch")
        .expect("pane id");
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let title = h.app.windows[h.app.active_window]
            .panes
            .get(&pane)
            .and_then(|pane| pane.as_view())
            .and_then(|view| match &view.runtime {
                crate::host::pane::ViewRuntime::Python(runtime) => Some(runtime.display_name()),
                _ => None,
            });
        if title.as_deref() == Some("nested") {
            break;
        }
        assert_ne!(title.as_deref(), Some("top"), "ran the root main.py");
        assert!(
            start.elapsed()
                < crate::testing::load_aware_timeout(std::time::Duration::from_secs(30)),
            "nested entry never ran (title {title:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// The admission row for a folder trust in `profile`, to revoke it.
fn trust_row(profile: &Path) -> String {
    PermissionMonitor::for_profile(profile)
        .unwrap()
        .list_entries()
        .into_iter()
        .find(|entry| entry.tool == TRUST_FOLDER_TOOL && entry.kind == "allow")
        .expect("a folder trust row")
        .id
}

/// The plugin-code ask waiting in `profile`'s Plexi window.
fn waiting_ask(profile: &Path) -> Vec<String> {
    PermissionMonitor::for_profile(profile)
        .unwrap()
        .list_pending()
        .into_iter()
        .filter(|pending| pending.tool == TRUST_FOLDER_TOOL || pending.tool == LOAD_INSTALLED_TOOL)
        .map(|pending| pending.pending_request_id)
        .collect()
}

/// Step 2: a cold scan finds an untrusted workspace plugin and its view with
/// zero asks. Opening it by id or by view id files the same folder ask. After
/// one Allow the open succeeds and starts exactly one guest.
#[test]
fn an_untrusted_plugin_is_discovered_and_opens_by_id_or_view_after_one_allow() {
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    let plugins = root
        .join(crate::config::workspace_channel_dir())
        .join("plugins");
    write_plugin(&plugins, "repo-tool");
    let registry = h.app.registries.rescan_root(&root);
    assert!(registry.get("repo-tool").is_some(), "discovered");
    assert!(
        waiting_ask(h.profile_dir()).is_empty(),
        "a scan asks nothing"
    );

    let starts = guest_starts_for_test();
    let by_view = h
        .app
        .launch_view_by_id_with_layout("repo-tool.main", None, &[], None)
        .unwrap_err();
    assert!(by_view.contains("are not trusted"), "{by_view}");
    let by_id = h
        .app
        .launch_view_by_id_with_layout("repo-tool", None, &[], None)
        .unwrap_err();
    assert!(by_id.contains("are not trusted"), "{by_id}");
    let asks = waiting_ask(h.profile_dir());
    assert_eq!(asks.len(), 1, "one ask for the folder: {asks:?}");
    assert!(by_view.contains(&asks[0]) && by_id.contains(&asks[0]));
    assert_eq!(guest_starts_for_test(), starts, "no guest before the Allow");

    PermissionMonitor::for_profile(h.profile_dir())
        .unwrap()
        .approve_pending(&asks[0], ApprovalChoice::Always)
        .unwrap();
    h.app
        .launch_view_by_id_with_layout("repo-tool.main", None, &[], None)
        .expect("opens after the Allow");
    assert_eq!(guest_starts_for_test(), starts + 1);
}

/// Step 3: revoke, then open on a warm registry. A guest started once; its
/// folder trust is revoked; the registry still lists it; a forced fresh pane
/// starts no guest. Python by id, WASM by id and the registered component by
/// path. Re-allowing restores the positive path.
#[test]
fn a_revoked_plugin_starts_no_guest_from_a_warm_registry() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let root = h.workspace_root();
    let plugins = root
        .join(crate::config::workspace_channel_dir())
        .join("plugins");
    write_plugin(&plugins, "py-tool");
    let wasm = write_wasm_plugin(&plugins, "wasm-tool");
    h.trust_plugin_folder(&plugins);
    h.app.registries.rescan_root(&root);
    for id in ["py-tool", "wasm-tool"] {
        let starts = guest_starts_for_test();
        h.app
            .launch_view_by_id_with_layout_forced(id, None, &[], None)
            .unwrap_or_else(|error| panic!("{id} opens while trusted: {error}"));
        assert_eq!(guest_starts_for_test(), starts + 1, "{id}");
    }

    let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
    assert!(monitor
        .revoke_grant_id(&trust_row(h.profile_dir()))
        .unwrap());
    let starts = guest_starts_for_test();
    for id in ["py-tool", "wasm-tool"] {
        assert!(
            h.app.registries.view_for_root(&root).get(id).is_some(),
            "warm"
        );
        let error = h
            .app
            .launch_view_by_id_with_layout_forced(id, None, &[], None)
            .unwrap_err();
        assert!(error.contains("are not trusted"), "{id}: {error}");
    }
    let component = wasm.join("component.wasm");
    let error = h
        .app
        .launch_view_by_path_with_layout(&component.to_string_lossy(), None, None, &[])
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert_eq!(guest_starts_for_test(), starts, "no guest after the revoke");

    h.trust_plugin_folder(&plugins);
    h.app
        .launch_view_by_id_with_layout_forced("py-tool", None, &[], None)
        .expect("re-allowed");
    assert_eq!(guest_starts_for_test(), starts + 1);
}

/// Step 3: the live constructors take a permit and nothing else names a plugin
/// for them; no code outside the admission module can build one. (A binary
/// crate has no compile-fail harness, so this pins the signatures and the
/// single construction site in source.)
#[test]
fn only_the_admission_module_builds_a_permit_and_constructors_require_one() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.ends_with("host/plugin_admission.rs")
                && !path.ends_with("host/plugin_admission_tests.rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("AdmittedPlugin {")
            {
                offenders.push(path);
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
    let python = std::fs::read_to_string(src.join("host/wasm_python.rs")).unwrap();
    assert!(python.contains(
        "pub(crate) fn launch(\n        admitted: crate::host::plugin_admission::AdmittedPlugin<'_>,"
    ));
    assert!(python.contains(
        "pub(crate) fn relaunch(\n        &mut self,\n        admitted: crate::host::plugin_admission::AdmittedPlugin<'_>,"
    ));
    let wasm = std::fs::read_to_string(src.join("host/wasm_view.rs")).unwrap();
    assert!(wasm.contains(
        "pub(crate) fn load_with_grants(\n        admitted: crate::host::plugin_admission::AdmittedPlugin<'_>,"
    ));
}

/// Step 4: revoke while the import review dialog waits, then accept it. The
/// accept goes to the same start: no guest starts and no import is granted.
/// With trust standing, the same accept starts exactly one guest with the
/// reviewed imports recorded.
#[test]
fn accepting_a_review_after_a_revoke_starts_nothing_and_grants_nothing() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let folder = tempfile::tempdir().unwrap();
    let component = folder.path().join("sysmon.wasm");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm"),
        &component,
    )
    .unwrap();
    h.trust_plugin_folder(folder.path());
    let launch = |h: &mut crate::testing::HostHarness| {
        h.app
            .launch_view_by_path_with_layout(&component.to_string_lossy(), None, None, &[])
            .expect("queues the review");
        h.app
            .pending_raw_wasm_launches
            .pop_front()
            .expect("an import review waits")
    };
    let review = launch(&mut h);
    assert!(!review.missing_capabilities.is_empty());

    let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
    assert!(monitor
        .revoke_grant_id(&trust_row(h.profile_dir()))
        .unwrap());
    let grant_rows = |monitor: &PermissionMonitor| {
        monitor
            .list_entries()
            .into_iter()
            .filter(|entry| entry.tool != TRUST_FOLDER_TOOL && entry.kind != "pending")
            .count()
    };
    let rows = grant_rows(&monitor);
    let store_file = h.profile_dir().join("permissions.toml");
    let store_before = std::fs::read(&store_file).ok();
    let starts = guest_starts_for_test();
    let evidence = crate::broker::consent::DecisionEvidence::for_test(&review.plugin_id, "allow");
    let error = h
        .app
        .open_wasm_view_pane(
            &review.plugin_id,
            &review.wasm_path,
            review.workspace_root.clone(),
            review.launch_args.clone(),
            Some((
                &review.missing_capabilities,
                evidence.admit(&review.plugin_id, "allow").unwrap(),
            )),
        )
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert_eq!(guest_starts_for_test(), starts, "no guest");
    assert_eq!(grant_rows(&monitor), rows, "no grant row added");
    assert!(
        monitor
            .list_entries()
            .iter()
            .all(|entry| !review.missing_capabilities.contains(&entry.tool)),
        "no reviewed import granted in the broker"
    );
    assert_eq!(
        std::fs::read(&store_file).ok(),
        store_before,
        "no legacy grant"
    );

    h.trust_plugin_folder(folder.path());
    let review = launch(&mut h);
    h.app
        .open_wasm_view_pane(
            &review.plugin_id,
            &review.wasm_path,
            review.workspace_root.clone(),
            review.launch_args.clone(),
            Some((
                &review.missing_capabilities,
                evidence.admit(&review.plugin_id, "allow").unwrap(),
            )),
        )
        .expect("starts with the reviewed imports");
    assert_eq!(guest_starts_for_test(), starts + 1);
    assert!(monitor
        .list_entries()
        .iter()
        .any(|entry| review.missing_capabilities.contains(&entry.tool)));
}

/// Step 5: a WASM development copy sharing the bundled singleton id `chess`
/// runs in its own pane. Opening `chess` by id starts (then focuses) the
/// bundled Python pane, never the dev pane, and opening the dev path again
/// focuses the dev pane, never the bundled one.
#[test]
fn a_wasm_dev_copy_and_the_bundled_singleton_keep_their_own_panes() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let profile_plugins = h.profile_dir().join("plugins");
    let bundled = extract_bundled("chess", &profile_plugins);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&profile_plugins);
    let dev = tempfile::tempdir().unwrap();
    let dev_chess = write_wasm_plugin(dev.path(), "chess");
    let manifest = std::fs::read_to_string(dev_chess.join("manifest.toml")).unwrap();
    std::fs::write(
        dev_chess.join("manifest.toml"),
        manifest.replace(
            "[runtime]",
            "[launch]\non_launch = \"focus_existing_in_context\"\n\n[runtime]",
        ),
    )
    .unwrap();
    h.trust_plugin_folder(dev.path());
    let workspace = h.workspace_root();
    let focused = |h: &crate::testing::HostHarness| {
        let win = &h.app.windows[h.app.active_window];
        win.focused_pane
            .and_then(|tile| win.tree.tiles.get_pane(&tile).copied())
    };
    let new_pane = |h: &mut crate::testing::HostHarness,
                    before: &[crate::spatial::tiling::PaneId]| {
        h.state()
            .open_panes
            .into_iter()
            .find(|pane| !before.contains(pane))
    };

    let before = h.state().open_panes;
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace.clone()))
        .expect("bundled chess");
    let bundled_pane = new_pane(&mut h, &before).expect("bundled pane");
    assert_eq!(
        pane_tree(&h, bundled_pane),
        crate::platform::path::canonical_or_self(&bundled)
    );

    let before = h.state().open_panes;
    let dev_pane = h
        .app
        .launch_view_by_path_with_layout(
            &dev_chess.to_string_lossy(),
            None,
            Some(workspace.clone()),
            &[],
        )
        .expect("dev chess from a trusted folder")
        .expect("dev pane id");
    assert_eq!(
        new_pane(&mut h, &before),
        Some(dev_pane),
        "a new pane, not the bundled one"
    );

    let before = h.state().open_panes;
    let again = h
        .app
        .launch_view_by_path_with_layout(
            &dev_chess.to_string_lossy(),
            None,
            Some(workspace.clone()),
            &[],
        )
        .expect("dev again")
        .expect("pane id");
    assert_eq!(again, dev_pane, "the dev path focuses its own pane");
    assert_eq!(h.state().open_panes, before);

    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace))
        .expect("chess by id again");
    assert_eq!(
        h.state().open_panes,
        before,
        "by id focuses an existing pane"
    );
    assert_eq!(focused(&h), Some(bundled_pane), "never the dev copy");
}

/// Step 5: origin matching walks the overlay chain. With the bundled singleton
/// buried under a WASM dev overlay, opening it by id reveals the bundled pane;
/// with the dev pane buried under the bundled overlay, opening the dev path
/// reveals the dev pane. Neither open starts a guest.
#[test]
fn a_buried_pane_is_revealed_only_for_its_own_origin() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let profile_plugins = h.profile_dir().join("plugins");
    extract_bundled("chess", &profile_plugins);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&profile_plugins);
    let dev = tempfile::tempdir().unwrap();
    let dev_chess = write_wasm_plugin(dev.path(), "chess");
    let manifest = std::fs::read_to_string(dev_chess.join("manifest.toml")).unwrap();
    std::fs::write(
        dev_chess.join("manifest.toml"),
        manifest.replace(
            "[runtime]",
            "[launch]\non_launch = \"focus_existing_in_context\"\n\n[runtime]",
        ),
    )
    .unwrap();
    h.trust_plugin_folder(dev.path());
    let workspace = h.workspace_root();
    let dev_path = dev_chess.to_string_lossy().into_owned();
    let top_origin = |h: &crate::testing::HostHarness| {
        let win = &h.app.windows[h.app.active_window];
        let pane = win
            .focused_pane
            .and_then(|tile| win.tree.tiles.get_pane(&tile).copied())
            .expect("a focused pane");
        win.panes
            .get(&pane)
            .and_then(|pane| pane.as_view())
            .and_then(|view| match view.runtime.origin() {
                crate::host::pane::PaneOrigin::Guest(origin) => Some(origin.entry().to_path_buf()),
                crate::host::pane::PaneOrigin::Builtin => None,
            })
            .expect("a guest pane")
    };
    let bundled_entry =
        crate::platform::path::canonical_or_self(&profile_plugins.join("chess").join("chess.py"));
    let dev_entry = crate::platform::path::canonical_or_self(&dev_chess.join("component.wasm"));

    // Bundled below, dev overlay on top: by id reveals the bundled pane.
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace.clone()))
        .expect("bundled chess");
    h.app
        .launch_view_by_path_with_layout(
            &dev_path,
            Some("overlay".to_string()),
            Some(workspace.clone()),
            &[],
        )
        .expect("dev overlay");
    assert_eq!(top_origin(&h), dev_entry, "the dev overlay is on top");
    let starts = guest_starts_for_test();
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace.clone()))
        .expect("reveal bundled");
    assert_eq!(top_origin(&h), bundled_entry);
    assert_eq!(guest_starts_for_test(), starts, "revealing starts no guest");

    // Dev below, bundled overlay on top: the dev path reveals the dev pane.
    h.app
        .launch_view_by_path_with_layout(&dev_path, None, Some(workspace.clone()), &[])
        .expect("dev pane");
    assert_eq!(top_origin(&h), dev_entry);
    h.app
        .launch_view_by_id_with_layout_forced(
            "chess",
            Some("overlay".to_string()),
            &[],
            Some(workspace.clone()),
        )
        .expect("bundled overlay over the dev pane");
    assert_eq!(top_origin(&h), bundled_entry);
    let starts = guest_starts_for_test();
    h.app
        .launch_view_by_path_with_layout(&dev_path, None, Some(workspace), &[])
        .expect("reveal dev");
    assert_eq!(top_origin(&h), dev_entry);
    assert_eq!(guest_starts_for_test(), starts, "revealing starts no guest");
}

/// Round 3, install path: a local install over a bundled id is refused before
/// it replaces the working copy, and the bundled plugin still starts.
#[test]
fn a_local_install_over_a_bundled_id_is_refused_and_the_copy_survives() {
    let rig = Rig::new();
    let _profile = crate::config::set_test_profile_dir(rig.profile.path().to_path_buf());
    let todo = extract_bundled("todo", &rig.plugins());
    let elsewhere = tempfile::tempdir().unwrap();
    let changed = write_plugin(elsewhere.path(), "todo");
    assert_eq!(
        crate::cli::plugin::plugin_install_with_pin(
            &changed.to_string_lossy(),
            None,
            crate::cli::InstallConfirm::Interactive,
            true,
        ),
        1
    );
    assert_eq!(
        rig.start(&todo),
        Ok(Basis::Bundled),
        "the bundled copy survives"
    );

    // On a case-insensitive volume `Todo` names the same folder as `todo`.
    let variant_parent = tempfile::tempdir().unwrap();
    let variant = write_plugin(variant_parent.path(), "Todo");
    assert_eq!(
        crate::cli::plugin::plugin_install_with_pin(
            &variant.to_string_lossy(),
            None,
            crate::cli::InstallConfirm::Interactive,
            true,
        ),
        1,
        "a case variant of a bundled id is refused"
    );
    assert_eq!(
        rig.start(&todo),
        Ok(Basis::Bundled),
        "the original keeps bundled admission"
    );

    // A personal id whose install path is a link to the bundled install.
    std::os::unix::fs::symlink(&todo, rig.plugins().join("mine")).unwrap();
    let linked_parent = tempfile::tempdir().unwrap();
    let linked = write_plugin(linked_parent.path(), "mine");
    assert_eq!(
        crate::cli::plugin::plugin_install_with_pin(
            &linked.to_string_lossy(),
            None,
            crate::cli::InstallConfirm::Interactive,
            true,
        ),
        1,
        "a destination that aliases a bundled install is refused"
    );
    assert_eq!(rig.start(&todo), Ok(Basis::Bundled));
}

/// Round 3, recovery: the command a changed copy's refusal names repairs it.
#[test]
fn the_refusal_hint_repairs_a_changed_bundled_copy() {
    let rig = Rig::new();
    let _profile = crate::config::set_test_profile_dir(rig.profile.path().to_path_buf());
    let todo = extract_bundled("todo", &rig.plugins());
    std::fs::write(todo.join("main.py"), "print('changed')\n").unwrap();
    let error = rig.start(&todo).unwrap_err();
    assert!(error.contains("plugin install local:todo"), "{error}");
    assert_eq!(crate::cli::install_cli("local:todo", true), 0);
    assert_eq!(rig.start(&todo), Ok(Basis::Bundled), "repaired");
}

/// Round 3, registry: a link at the profile's bundled id cannot supply the
/// owner, even to a trusted development tree. The id and its views stay out of
/// the index and no guest starts.
#[test]
fn a_profile_link_cannot_own_a_bundled_id_or_its_views() {
    let mut h = crate::testing::HostHarness::new();
    let plugins = h.profile_dir().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let dev = tempfile::tempdir().unwrap();
    let tree = extract_bundled("todo", dev.path());
    std::fs::write(tree.join("main.py"), "print('linked')\n").unwrap();
    h.trust_plugin_folder(dev.path());
    std::os::unix::fs::symlink(&tree, plugins.join("todo")).unwrap();
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    let root = h.workspace_root();
    let view = h.app.registries.view_for_root(&root);
    assert!(view.get("todo").is_none(), "a link owns nothing");
    assert!(
        view.list().iter().all(|plugin| plugin
            .contributes
            .views
            .iter()
            .all(|v| v.id != "todo.main")),
        "nor its views"
    );
    let starts = guest_starts_for_test();
    assert!(h
        .app
        .launch_view_by_id_with_layout("todo", None, &[], None)
        .is_err());
    assert_eq!(guest_starts_for_test(), starts, "no guest");
}

/// Round 3, registry: a duplicate global record declaring a bundled id never
/// displaces the verified record, whichever the directory scan meets last.
#[test]
fn a_duplicate_global_record_cannot_own_a_bundled_id() {
    let rig = Rig::new();
    let genuine = extract_bundled("todo", &rig.plugins());
    for name in ["0-duplicate", "zz-duplicate"] {
        let duplicate = rig.plugins().join(name);
        std::fs::create_dir_all(&duplicate).unwrap();
        for file in ["manifest.toml"] {
            std::fs::copy(genuine.join(file), duplicate.join(file)).unwrap();
        }
        let entry = crate::app::registry::PluginRegistry::empty()
            .load_plugin(&genuine)
            .unwrap()
            .manifest
            .entry;
        std::fs::write(duplicate.join(entry), "print('duplicate')\n").unwrap();
    }
    let registry = rig.load();
    let todo = registry.get("todo").expect("the verified owner");
    assert_eq!(
        crate::platform::path::canonical_or_self(&todo.plugin_dir),
        crate::platform::path::canonical_or_self(&genuine)
    );
}

/// A changed copy at the bundled root is not indexed; its load error names the
/// repair, and it can still be uninstalled.
#[test]
fn a_changed_bundled_copy_reports_its_repair_and_can_be_uninstalled() {
    let rig = Rig::new();
    let _profile = crate::config::set_test_profile_dir(rig.profile.path().to_path_buf());
    let todo = extract_bundled("todo", &rig.plugins());
    std::fs::write(todo.join("main.py"), "print('changed')\n").unwrap();
    let registry = rig.load();
    assert!(registry.get("todo").is_none());
    let skipped = registry.skipped_reason("todo").expect("reported");
    assert!(
        skipped.error.contains("plugin install local:todo"),
        "{}",
        skipped.error
    );
    assert_eq!(crate::cli::plugin::plugin_uninstall("todo", true), 0);
    assert!(!todo.exists());
}

// ── Stint 1733: a live pane carries the owner admission verified ──────────

/// The view pane `pane` names, wherever it is.
fn view_pane(
    h: &crate::testing::HostHarness,
    pane: crate::spatial::tiling::PaneId,
) -> &crate::host::pane::ViewPane {
    h.app
        .windows
        .iter()
        .find_map(|window| window.panes.get(&pane))
        .and_then(|pane| pane.as_view())
        .expect("a view pane")
}

/// The guest origin of `pane`; panics for a builtin.
fn guest_origin(
    h: &crate::testing::HostHarness,
    pane: crate::spatial::tiling::PaneId,
) -> GuestOrigin {
    match view_pane(h, pane).runtime.origin() {
        crate::host::pane::PaneOrigin::Guest(origin) => origin.clone(),
        crate::host::pane::PaneOrigin::Builtin => panic!("pane {pane} is a builtin"),
    }
}

/// Run `launch` and return the pane it opened.
fn opened_by(
    h: &mut crate::testing::HostHarness,
    launch: impl FnOnce(&mut crate::testing::HostHarness),
) -> crate::spatial::tiling::PaneId {
    let before = h.state().open_panes;
    launch(h);
    h.state()
        .open_panes
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("the launch opens a pane")
}

/// Path-launch the plugin folder `dir` in the harness workspace.
fn open_path(h: &mut crate::testing::HostHarness, dir: &Path) -> crate::spatial::tiling::PaneId {
    let workspace = h.workspace_root();
    opened_by(h, |h| {
        h.app
            .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, Some(workspace), &[])
            .expect("path launch");
    })
}

/// Open `id` by id in the harness workspace, in a split so it gets its own
/// pane id (an overlay reuses the id of the pane it covers).
fn open_id(h: &mut crate::testing::HostHarness, id: &str) -> crate::spatial::tiling::PaneId {
    let workspace = h.workspace_root();
    opened_by(h, |h| {
        h.app
            .launch_view_by_id_with_layout(id, Some("split_h".to_string()), &[], Some(workspace))
            .expect("id launch");
    })
}

/// A harness whose profile plugins folder holds the bundled `ids`, indexed.
fn harness_with_bundled(ids: &[&str]) -> (crate::testing::HostHarness, PathBuf) {
    let mut h = crate::testing::HostHarness::new();
    let plugins = h.profile_dir().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    for id in ids {
        extract_bundled(id, &plugins);
    }
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    (h, plugins)
}

/// Capability rows stored for plugin `id` (grants, not admissions or asks).
fn capability_rows(profile: &Path, id: &str) -> usize {
    PermissionMonitor::for_profile(profile)
        .unwrap()
        .list_entries()
        .into_iter()
        .filter(|entry| {
            entry.actor_id.ends_with(id)
                && entry.kind != "pending"
                && entry.tool != TRUST_FOLDER_TOOL
                && entry.tool != LOAD_INSTALLED_TOOL
        })
        .count()
}

/// A Python plugin `id` declaring `fs.read`, with an optional `[launch]`.
fn write_capable_plugin(parent: &Path, id: &str, launch: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[plugin.capabilities]\ncapabilities = [\"fs.read\"]\n\n{launch}[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("main.py"),
        "from plexi_sdk.ui import Column, Text\n\ndef init(size, args):\n    return []\n\ndef update(event):\n    return []\n\ndef view():\n    return Column([Text('t1733')])\n",
    )
    .unwrap();
    dir
}

/// Scope 1, on the real route: the Permissions copy at its profile root is
/// admitted as bundled; a byte-identical copy in a trusted folder is admitted
/// by folder trust, so its pane carries a development origin.
#[test]
fn a_folder_copy_of_permissions_carries_a_development_origin() {
    let (mut h, plugins) = harness_with_bundled(&["permissions"]);
    let bundled = open_id(&mut h, "permissions");
    let origin = guest_origin(&h, bundled);
    assert!(origin.is_bundled(), "{origin:?}");
    assert_eq!(
        origin.root(),
        crate::platform::path::canonical_or_self(&plugins.join("permissions"))
    );

    let dev = tempfile::tempdir().unwrap();
    let copy = extract_bundled("permissions", dev.path());
    assert!(crate::cli::install_host::matches_bundled_plugin(
        "permissions",
        &copy
    ));
    h.trust_plugin_folder(dev.path());
    let dev_pane = open_path(&mut h, &copy);
    let origin = guest_origin(&h, dev_pane);
    assert_eq!(origin.basis(), Basis::TrustedFolder);
    assert!(!origin.is_bundled(), "identical bytes are not provenance");
    assert_eq!(
        view_pane(&h, dev_pane).tool_identity(),
        format!("dev:permissions:{dev_pane}")
    );
    assert_eq!(view_pane(&h, bundled).tool_identity(), "permissions");
}

/// Scope 2: a development pane declaring a bundled id publishes its tools
/// only under a pane-qualified name; the bundled tool name never resolves to
/// it, alone or beside the bundled pane.
#[test]
fn a_dev_copy_publishes_its_tools_only_under_a_pane_qualified_name() {
    let (mut h, _plugins) = harness_with_bundled(&["chess"]);
    let dev = tempfile::tempdir().unwrap();
    let dev_chess = extract_bundled("chess", dev.path());
    h.trust_plugin_folder(dev.path());
    let context = h.app.windows[h.app.active_window].context_id;
    let names = |h: &crate::testing::HostHarness| {
        let scope = crate::plexi_ai::tool_dispatch::DispatchScope::new(
            0,
            "agent:t1733",
            h.workspace_root(),
            context,
        );
        let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        crate::plexi_ai::tool_dispatch::ToolDispatcher::namespaced_for(scope, monitor)
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };
    let registered_as = |pane| {
        crate::plexi_ai::tool_dispatch::live_exposed_tools()
            .into_iter()
            .filter(|(_, id, _, _)| *id == pane)
            .map(|(plugin, _, tool, _)| (plugin, tool))
            .collect::<Vec<_>>()
    };

    let dev_pane = open_path(&mut h, &dev_chess);
    h.wait_for_exposed_tools(dev_pane);
    let dev_tools = registered_as(dev_pane);
    assert!(!dev_tools.is_empty());
    assert!(
        dev_tools
            .iter()
            .all(|(plugin, _)| *plugin == format!("dev:chess:{dev_pane}")),
        "{dev_tools:?}"
    );
    let tool = dev_tools[0].1.clone();
    let alone = names(&h);
    assert!(
        alone.contains(&format!("dev:chess:{dev_pane}__{tool}")),
        "{alone:?}"
    );
    assert!(
        !alone.contains(&format!("chess__{tool}")),
        "the bundled name never resolves to the dev pane: {alone:?}"
    );

    let bundled_pane = open_id(&mut h, "chess");
    h.wait_for_exposed_tools(bundled_pane);
    assert!(registered_as(bundled_pane)
        .iter()
        .all(|(plugin, _)| plugin == "chess"));
    let both = names(&h);
    assert!(both.contains(&format!("chess__{tool}")), "{both:?}");
    assert!(
        both.contains(&format!("dev:chess:{dev_pane}__{tool}")),
        "{both:?}"
    );
}

/// Scope 3: the origin is frozen when the guest starts. Replacing the dev
/// folder with a link to the profile install afterwards does not make the
/// running dev pane the bundled plugin's pane.
#[test]
fn a_later_link_change_cannot_change_a_running_guests_origin() {
    let (mut h, plugins) = harness_with_bundled(&["logs"]);
    let bundled_entry = crate::app::registry::resolve_plugin_dir(&plugins.join("logs"))
        .unwrap()
        .entry;
    let dev = tempfile::tempdir().unwrap();
    let a = dev.path().join("a");
    let dev_logs = extract_bundled("logs", &a);
    h.trust_plugin_folder(&a);
    let pane = open_path(&mut h, &dev_logs);
    let before = guest_origin(&h, pane);

    std::fs::rename(&a, dev.path().join("old")).unwrap();
    std::os::unix::fs::symlink(&plugins, &a).unwrap();
    assert_eq!(
        crate::platform::path::canonical_or_self(before.entry()),
        bundled_entry,
        "the old entry path now resolves to the bundled install"
    );
    assert_eq!(guest_origin(&h, pane), before, "the origin did not move");
    assert!(
        h.app
            .locate_view_instance(
                "logs",
                None,
                crate::pane_ops::create::OriginQuery::Entry(&bundled_entry),
            )
            .is_none(),
        "the dev pane never stands in for the bundled one"
    );
}

/// Scope 3: a builtin open never consults registry metadata first. A
/// personal plugin named `text-editor` declaring `focus_existing` cannot
/// satisfy the Text Editor open with its own pane.
#[test]
fn opening_text_editor_never_matches_a_colliding_registry_singleton() {
    let mut h = crate::testing::HostHarness::new();
    let plugins = h.profile_dir().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let impostor = write_capable_plugin(
        &plugins,
        "text-editor",
        "[launch]\non_launch = \"focus_existing\"\n\n",
    );
    PluginAdmission::current().allow_copy_for_test("text-editor", &impostor);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    let guest = open_path(&mut h, &impostor);
    assert_eq!(guest_origin(&h, guest).owner(), "text-editor");

    let editor = open_id(&mut h, "text-editor");
    assert_ne!(editor, guest);
    assert_eq!(
        view_pane(&h, editor).runtime.origin(),
        crate::host::pane::PaneOrigin::Builtin
    );
}

/// Scope 4: a bundled plugin's contributed view names are reserved like its
/// id. A personal plugin whose id is `todo.main` does not capture the open
/// of the bundled Todo view.
#[test]
fn a_personal_plugin_named_after_a_bundled_view_does_not_capture_its_open() {
    let (mut h, plugins) = harness_with_bundled(&["todo"]);
    let personal = write_plugin(&plugins, "todo.main");
    PluginAdmission::current().allow_copy_for_test("todo.main", &personal);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    let pane = open_id(&mut h, "todo.main");
    let origin = guest_origin(&h, pane);
    assert_eq!(origin.owner(), "todo");
    assert!(origin.is_bundled());
}

/// Scopes 5 and 6: an open the admission refuses leaves no capability rows,
/// by path or by id; an id open whose folder now holds another plugin is
/// refused before anything is granted under either id.
#[test]
fn a_refused_plugin_keeps_no_capability_rows() {
    let mut h = crate::testing::HostHarness::new();
    let untrusted = tempfile::tempdir().unwrap();
    let dir = write_capable_plugin(untrusted.path(), "t1733-refused", "");
    let starts = guest_starts_for_test();
    let error = h
        .app
        .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, None, &[])
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert_eq!(capability_rows(h.profile_dir(), "t1733-refused"), 0);

    let plugins = h.profile_dir().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    write_capable_plugin(&plugins, "t1733-unallowed", "");
    let moved = write_capable_plugin(&plugins, "t1733-moved", "");
    PluginAdmission::current().allow_copy_for_test("t1733-moved", &moved);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&plugins);
    let workspace = h.workspace_root();
    let error = h
        .app
        .launch_view_by_id_with_layout("t1733-unallowed", None, &[], Some(workspace.clone()))
        .unwrap_err();
    assert!(error.contains("has not been allowed"), "{error}");
    assert_eq!(capability_rows(h.profile_dir(), "t1733-unallowed"), 0);

    let manifest = moved.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("\"t1733-moved\"", "\"t1733-other\""),
    )
    .unwrap();
    let error = h
        .app
        .launch_view_by_id_with_layout("t1733-moved", None, &[], Some(workspace))
        .unwrap_err();
    assert!(error.contains("t1733-other"), "{error}");
    assert_eq!(capability_rows(h.profile_dir(), "t1733-moved"), 0);
    assert_eq!(capability_rows(h.profile_dir(), "t1733-other"), 0);
    assert_eq!(guest_starts_for_test(), starts, "no guest");
}

/// Scope 5: the launch config comes from the manifest the resolver parsed;
/// the file is not read a second time.
#[test]
fn the_launch_config_comes_from_the_resolved_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = write_capable_plugin(dir.path(), "t1733-parse", "");
    let resolved = crate::app::registry::resolve_plugin_dir(&plugin).unwrap();
    std::fs::write(plugin.join("manifest.toml"), "not a manifest").unwrap();
    let config = crate::host::wasm_python::PythonLaunchConfig::from_resolved(&resolved)
        .expect("no second read")
        .expect("a Python plugin");
    assert_eq!(config.plugin_id, "t1733-parse");
    assert_eq!(config.declared_capabilities, vec!["fs.read".to_string()]);
    assert_eq!(config.entry, resolved.entry);
}

/// Scope 7: when a reviewed raw component fails to instantiate, the imports
/// the review allowed are not recorded in either store.
#[test]
fn a_failed_instantiation_after_review_leaves_no_import_grants() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let folder = tempfile::tempdir().unwrap();
    let component = folder.path().join("unlinkable.wasm");
    // Imports host state with a shape the host does not provide: inspection
    // asks for the state import, linking then fails. Wasmtime reads the text
    // format from a file.
    std::fs::write(
        &component,
        "(component (import \"plexi:platform/host-state@9.9.9\" (instance (export \"t1733\" (func)))))",
    )
    .unwrap();
    h.trust_plugin_folder(folder.path());
    h.app
        .launch_view_by_path_with_layout(&component.to_string_lossy(), None, None, &[])
        .expect("queues the review");
    let review = h
        .app
        .pending_raw_wasm_launches
        .pop_front()
        .expect("an import review waits");
    assert!(!review.missing_capabilities.is_empty());
    let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
    let granted = |monitor: &PermissionMonitor| {
        monitor
            .list_entries()
            .into_iter()
            .filter(|entry| review.missing_capabilities.contains(&entry.tool))
            .count()
    };
    let store_file = h.profile_dir().join("permissions.toml");
    let store_before = std::fs::read(&store_file).ok();
    let evidence = crate::broker::consent::DecisionEvidence::for_test(&review.plugin_id, "allow");
    let error = h
        .app
        .open_wasm_view_pane(
            &review.plugin_id,
            &review.wasm_path,
            review.workspace_root.clone(),
            review.launch_args.clone(),
            Some((
                &review.missing_capabilities,
                evidence.admit(&review.plugin_id, "allow").unwrap(),
            )),
        )
        .unwrap_err();
    assert!(error.contains("load"), "{error}");
    assert_eq!(granted(&monitor), 0, "no reviewed import granted");
    assert_eq!(
        std::fs::read(&store_file).ok(),
        store_before,
        "no legacy grant"
    );
}

/// The pane holding window focus.
fn focused_pane(h: &crate::testing::HostHarness) -> crate::spatial::tiling::PaneId {
    let window = &h.app.windows[h.app.active_window];
    window
        .focused_pane
        .and_then(|tile| window.tree.tiles.get_pane(&tile).copied())
        .expect("a focused pane")
}

/// Codex 3309 round 1: bundled-id reuse needs a bundled origin, never a path
/// match. After the profile install of Logs is swapped for a link to a
/// running dev copy, the registry's cached entry resolves to the dev pane's
/// entry; opening Logs by id still focuses the bundled pane.
#[test]
fn bundled_id_reuse_never_focuses_a_dev_pane_through_a_swapped_install() {
    let (mut h, plugins) = harness_with_bundled(&["logs"]);
    let bundled = open_id(&mut h, "logs");
    let dev = tempfile::tempdir().unwrap();
    let dev_logs = extract_bundled("logs", dev.path());
    h.trust_plugin_folder(dev.path());
    let dev_pane = open_path(&mut h, &dev_logs);
    assert_eq!(focused_pane(&h), dev_pane);

    let install = plugins.join("logs");
    std::fs::rename(&install, plugins.join("logs.moved")).unwrap();
    std::os::unix::fs::symlink(&dev_logs, &install).unwrap();
    let cached = h
        .app
        .registries
        .view_for_root(&h.workspace_root())
        .get("logs")
        .expect("the cached bundled entry")
        .bin_path
        .clone();
    assert_eq!(
        crate::platform::path::canonical_or_self(&cached),
        guest_origin(&h, dev_pane).entry(),
        "the cached entry now resolves to the dev pane's entry"
    );

    let workspace = h.workspace_root();
    let focused = h
        .app
        .launch_view_by_id_with_layout("logs", None, &[], Some(workspace))
        .expect("logs by id");
    assert_eq!(focused, Some(bundled), "the bundled pane, never the dev one");
    assert_eq!(focused_pane(&h), bundled);
}

/// Codex 3309 round 1: a fresh id open of a bundled id starts only the
/// verified bundled copy. A link at its install root to a trusted dev tree
/// is refused, not admitted by folder trust under the bundled name.
#[test]
fn a_fresh_id_open_of_a_bundled_id_never_starts_a_linked_dev_tree() {
    let (mut h, plugins) = harness_with_bundled(&["logs"]);
    let dev = tempfile::tempdir().unwrap();
    let dev_logs = extract_bundled("logs", dev.path());
    h.trust_plugin_folder(dev.path());
    let install = plugins.join("logs");
    std::fs::rename(&install, plugins.join("logs.moved")).unwrap();
    std::os::unix::fs::symlink(&dev_logs, &install).unwrap();
    let starts = guest_starts_for_test();
    let workspace = h.workspace_root();
    let error = h
        .app
        .launch_view_by_id_with_layout("logs", Some("split_h".to_string()), &[], Some(workspace))
        .unwrap_err();
    assert!(error.contains("bundled with Plexi"), "{error}");
    assert_eq!(guest_starts_for_test(), starts, "no guest");
}

/// Codex 3309 round 1: a manifest-backed WASM plugin admitted by folder
/// trust that fails to instantiate leaves none of its manifest capability
/// rows behind.
#[test]
fn a_manifest_wasm_plugin_that_fails_to_start_keeps_no_capability_rows() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let mut h = crate::testing::HostHarness::new();
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path().join("t1733-unlinkable");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("component.wasm"),
        "(component (import \"plexi:platform/host-state@9.9.9\" (instance (export \"t1733\" (func)))))",
    )
    .unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        "schema_version = 2\n\n[plugin]\nid = \"t1733-unlinkable\"\nname = \"t\"\nversion = \"0.0.1\"\n\n[plugin.capabilities]\ncapabilities = [\"timer\"]\n\n[runtime]\nkind = \"wasm\"\nentry = \"component.wasm\"\n\n[[contributes.views]]\nid = \"t1733-unlinkable.main\"\ndefault = true\n",
    )
    .unwrap();
    h.trust_plugin_folder(folder.path());
    let error = h
        .app
        .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, None, &[])
        .unwrap_err();
    assert!(error.contains("load"), "{error}");
    assert_eq!(capability_rows(h.profile_dir(), "t1733-unlinkable"), 0);
}
