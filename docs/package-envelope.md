# Package Envelope: One Format for Apps, Agents, and Skills

Status: active.
Stint: 0325.
Parent: [`app-framework-marketplace.md`](app-framework-marketplace.md).

The roadmap says one marketplace package can carry an app, an agent, or a skill. Today only apps have a package: `plexi app package` writes a `.plexipkg`, `plexi app validate` checks it, and `plexi app install` shows a trust sheet before it lands. Agents and skills reach the disk by other routes. `plexi agent add` copies an `AGENT.md` out of the global registry into a workspace; skills arrive through the external agent-framework installer that owns `.agents/skills/`. Neither is validated, capability-labelled, or scope-checked the way an app is.

This spec defines the envelope that closes that gap: the single package format all three kinds share, the fields the trust sheet reads, where each kind installs, and the validation rules that keep a package's declared kind, powers, scope, and trust label from disagreeing. It specifies the contract. It does not build install support for agents or skills (that is out of scope, see the end).

The app package is the base. Everything here is an addition to the schema in [`src/app/package.rs`](../src/app/package.rs) and [`src/app/registry.rs`](../src/app/registry.rs), never a second schema beside it.

## The Envelope Is the App Package Plus a Kind

A Plexi package is already fully defined: the `PACKAGE.toml` descriptor (`schema_version`, `[package]` with `id`/`version`/`runtime`, a `capabilities` list, and a `files` list of path + sha256 + size), the `manifest.toml` it wraps, and the app source. `validate_package` parses it into a `PackageReport`. `app-framework-marketplace.md` §3 lists what validation must reject. None of that changes.

The envelope adds exactly one discriminator: a package `kind`.

```toml
# PACKAGE.toml
[package]
id = "standup-notes"
version = "1.2.0"
runtime = "python"
kind = "app"        # app | agent | skill — optional, defaults to "app"
```

`kind` is optional and defaults to `app`. A descriptor that omits it is an app package, so every package that validates today keeps validating unchanged. This is a real default, not a compatibility shim: absence has one meaning, and that meaning is the common case. The three kinds are not three formats. They are one format with a field that tells the host how to place the payload and which extra validation rules apply.

Each kind constrains the base package. `app` is the unconstrained form. `agent` and `skill` are narrower: they carry a specific payload shape and, for skills, forfeit the right to declare capabilities at all. The rest of this document is those constraints.

## Identity

Package identity is the manifest's identity. `AppManifestApp` already carries `id`, `name`, `version`, `author`, `repo`, `tags`, and `description` (see `registry.rs`); `PackageReport` surfaces `id`, `name`, `version`, and `runtime` after validation. The envelope adds no identity fields. A package is identified by `(kind, id, version)`. Two packages of different kinds may not share an id. An `agent` named `researcher` and a `skill` named `researcher` are a collision the validator rejects, because a user typing `researcher` must resolve to one thing.

Versioning follows the manifest's existing rules across all kinds. `[requires]` (`plexi_min`, `plexi_max` in `RequiresSection`) is the host-compatibility floor and ceiling; `[app] min_sdk_version` gates the SDK. These apply to any kind that runs code. A `skill` package that ships no runtime of its own still declares `[requires]` when its instructions assume a host version, and the validator carries it into `PackageReport.requires_plexi_min` exactly as it does for apps.

## Install Scope Is an Install Flag, Never a Manifest Field

Every package installs at one of two scopes: workspace or global. Workspace is the default; the nearest `.plexi/` wins. Global is opt-in with `--global` (the conceptual `-ws` / `-g` split). This is already how `plexi app init` and `plexi agent add` behave, and it holds for all three kinds.

The manifest must never pin scope. Scope is the installer's decision at install time, made by the person running the command, not a property the author baked in. A package that tried to force its own scope would take a choice that belongs to the user. The validator rejects any scope field in `PACKAGE.toml` or `manifest.toml`.

Where each kind lands:

- **app** — the channel apps dir for global, or `.plexi/apps/<id>/` for workspace. Unchanged from today.
- **agent** — `<config_dir>/agents/<name>/` for global (the existing agent registry root), or `.plexi/agents/<name>/` for workspace. `plexi agent add` already writes the workspace side by copying from the global registry; the envelope makes that global registry a place a package can install into, rather than a directory a user hand-populates.
- **skill** — `<config_dir>/skills/<id>/` for global, `.plexi/skills/<id>/` for workspace. A skill bundled inside an app or agent installs wherever its host installs and takes the host's scope; it is not separately scoped.

## The Three Kinds

### app

The base kind, specified in full by `app-framework-marketplace.md` §3 and enforced by `package.rs`. An app is a runnable PGAP or WASM process with its own capability set and trust label. Nothing in this document overrides that. The other two kinds are defined by how they differ from it.

### agent

An agent exists in two forms today, and the envelope keeps both:

- **The runtime** is a PGAP app. `plexi agent init` scaffolds a `python_agent` app with `ai.query`; it packages, validates, and installs as `kind = "app"` like any other. An agent that *runs* is an app, and rides the app envelope with no special case.
- **The persona** is the definition that steers a runtime: an `AGENT.md`, optional bundled skills, and the workspace scaffold (`memory/`, `logs/`) that `plexi agent add` creates. This is what `kind = "agent"` packages.

So the answer to "are agents installable packages or registry entries" is both, and the two are the same act. An `agent` package's install *target* is the agent registry — `<config_dir>/agents/` globally or `.plexi/agents/` in a workspace. Installing the package is what puts an entry in the registry; the marketplace is how a persona travels between machines, and the registry is where it lives once it arrives. A local persona a user wrote by hand is an unpackaged registry entry, exactly as a workspace-local app is an unpackaged app.

An `agent` package requires an `AGENT.md` as its entry. It declares no runtime capabilities of its own, because the persona does not execute; the runtime app it names does, under that app's manifest. If a persona ships bundled skills, they follow the skill rules below.

### skill

A skill is instructions plus assets: a `SKILL.md` and whatever files it references. It has no process and no independent authority. A skill runs inside a host — the app or agent that invokes it — and every action it takes is an action by that host, under the host's granted capabilities.

The consequence is the load-bearing rule of this spec: **a skill package declares no capabilities.** A `skill` `PACKAGE.toml` with a non-empty `capabilities` list is invalid and the validator rejects it. A skill cannot hold `net.http` or `terminal.bindings`, because there is no skill process to grant them to. Whatever the skill's text tells a host to do, the host may do only if the host's own manifest declared the capability. This is what stops a skill from becoming a capability-laundering path: bundling a skill into an app can never widen that app's powers.

## Bundling and Inherited Permissions

An app or an agent persona may bundle skills — ship them inside its package so they install and scope with their host. A bundled skill is not separately installed and not separately scoped: it lives at its host's scope and disappears when the host is removed. A skill bundled in a workspace-installed app is invisible globally; a skill in a global agent is available everywhere that agent is.

Bundled skills inherit, they do not request. A bundled skill operates strictly within the host manifest's declared capability set — the `Capability` values in `AppCapabilities`, the same set the trust sheet shows. It cannot inherit more than the host declared and cannot ask for a capability the host omitted. There is no per-skill grant and no per-skill consent prompt; the host's grants are the only grants. If a skill needs `net.http`, the host app must declare `net.http`, the user must see it on the host's trust sheet, and the user's consent to the host is consent for everything the host's skills do with it.

This keeps the trust sheet honest. A user reviewing an app sees one capability list, and that list bounds the app and every skill it carries. A skill can never be the reason an installed thing did something its trust sheet did not disclose.

## Validation: Kind, Powers, Scope, and Trust Label Must Agree

Validation extends `validate_package` (`package.rs`) rather than forking it. Every check that runs today — unknown capability strings against `Capability::all_str_values()`, unsupported runtime, missing entry, files outside the package root, symlink or path-traversal escape, descriptor metadata that does not match contents, reviewed-native bypass patterns — runs for every kind. The envelope adds a consistency layer on top so that a package cannot claim one thing in its kind and another in its powers, scope, or label.

The rules:

1. **`kind` must be one of `app`, `agent`, `skill`.** Absent means `app`. Any other string is rejected, the same way an unknown capability is.
2. **A `skill` package declares no capabilities.** A non-empty `capabilities` list on `kind = "skill"` is rejected. Skills hold no grants (see above).
3. **An `agent` package's entry is an `AGENT.md`, and it declares no runtime capabilities.** The persona does not execute; its runtime app carries the powers.
4. **The manifest pins no scope.** A scope field anywhere in the package is rejected. Scope is the installer's, not the author's.
5. **The trust label is computed, never declared.** The label comes from `trust_label(runtime, is_first_party, marketplace_reviewed)` in `package.rs` — `FirstPartyCore`, `ReviewedNative`, `PythonUnreviewed`, `NativeUnreviewed`, or `SandboxedWasm`, each with the blunt `display_str` that never claims a sandbox that does not exist. A package that ships its own trust label is rejected; the host derives it from runtime and review state and shows its own conclusion, exactly as `RegistryEntry::from_report` already does. A skill or agent persona, having no runtime process, shows its host's trust posture, not one of its own.

The invariant these rules protect: for any installed thing, its kind, the capabilities its trust sheet shows, the scope it was installed at, and the trust label the host computed all describe the same reality. There is no combination where the label says less than the powers, or the kind implies authority the package does not actually hold.

## Non-Goals

Set by stint 0325, restated only as a boundary:

- No install implementation for `agent` or `skill` packages. This is the contract those installers must satisfy, not the installers.
- No payment or licensing mechanics. The commercial model — gated downloads, no client-side license, update-gating — lives in [`marketplace-monetization.md`](marketplace-monetization.md) and governs all kinds without restatement here.
- No replacement of the existing local skill installer. The envelope defines how a skill *package* is shaped and validated; the mechanism that writes `SKILL.md` to disk is unchanged.

## Source-of-Truth Rules

- This spec owns the package `kind` discriminator, the per-kind install targets, the skill "no capabilities / inherit from host" rule, and the cross-kind validation consistency layer.
- The base package format, the app trust sheet, and the reject list are owned by `app-framework-marketplace.md` §3 and the code in `src/app/package.rs`. This spec references them and never restates them.
- The trust boundary (consent + audit, no sandbox for native apps) is owned by [`src/process_app/SECURITY_MODEL.md`](../src/process_app/SECURITY_MODEL.md).
- The canonical capability set is the `Capability` enum in `src/app/permissions.rs`. New capabilities are added there, not here.
- Commercial behavior (purchase, update-gating, refunds) is owned by `marketplace-monetization.md`.
- When a future change alters a decision here, update this spec in the same PR, and delete it in the PR that closes stint 0325's successor build tasks once the envelope is implemented.
