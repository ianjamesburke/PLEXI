# Distribution contract

The distribution library (`crates/distribution`) owns package validation,
release selection, installation receipts, activation, recovery, rollback and
removal. `scripts/package.py` assembles packages for both source installation
and release publication. The shell and PowerShell bootstraps download the
released native installer and verify its SHA-256 before execution.

## Public installation

The public default is stable:

```sh
curl -fsSL https://plexiapp.com/install | bash
```

```powershell
irm https://plexiapp.com/install.ps1 | iex
```

The bootstrap resolves the latest stable release once and pins its installer
and checksum downloads to that tag. `PLEXI_BOOTSTRAP_TAG` can pin a particular
released installer. Neither public entrypoint downloads executable code from
a development branch. Deploy these website entrypoints only after the first
package-based release has passed publication gates and contains the installer
assets; a code merge alone does not publish those assets.

Use `--channel alpha` or `--channel beta` with the Unix command, or `-Channel`
with PowerShell, to select a prerelease channel. `--install-only` / `-InstallOnly`
verifies and installs without launching a host. The default runs the absolute
installed executable's `host start` and waits for readiness. The current shell
may need the printed PATH command; new shells read the idempotent PATH block.

`PLEXI_INSTALL_DIR` and `PLEXI_BIN_DIR` select custom installation and command
roots. Their canonical paths are persisted in the receipt and reused for
updates. Source builds use `scripts/install.sh --from-source [channel]`, which
builds and feeds the same package to the same transaction. It requires Rust,
just, uv and Python 3.11 or newer.

## Package

`package.json` schema 1 records the source commit, compiled build identity,
version, tag, channel, platform, executable, resource root and SHA-256 inventory.
Every package contains the host, native installer/launcher, Python SDK, WASI
interpreter, standard library and Calculator runtime probe. Resources resolve
from the executable's package; release execution never invokes checkout scripts.
Native Python is not required for the WASI app runtime. User-authored external
commands may have their own system dependencies.

The builder assembles channel-specific Mac bundle names, executable names and
identifiers before signing. A stable release contains separate alpha and beta
variants so those channels can consume it without changing a signed bundle.
`PLEXI_SIGN_IDENTITY` selects an available signing identity; the default is
ad-hoc signing. Public builds currently lack Developer ID notarization and
Windows Authenticode signing. Broad nontechnical distribution still needs those
credentials and clean-machine OS trust validation.

## Layout and ownership

macOS uses per-user `~/Applications/Plexi.app`, `Plexi Alpha.app` and
`Plexi Beta.app` links to immutable generations. Linux uses an XDG data
installation with a command launcher, desktop entry and packaged icon. Windows
uses LocalAppData, versioned host executables, a stable launcher, user PATH
registration and a Start Menu shortcut. A command launcher contains no second
copy of the host binary.

`installation.json` lives beside `generations/`, outside the user profile. It
records the active and previous generations, canonical destinations and owned
integrations. The per-user installation registry locates custom destinations.
The active package owns executable identity; a profile `installed_tag` marker
cannot identify an executable. SDK and core-app refresh stamps use the compiled
build identity, including source changes between prerelease builds.

Bare `plexi` delegates to the pane's named channel only when `PLEXI_RUNNING=1`.
Outside a pane it launches stable. Channel-named commands retain their channel.
`host start`, update, restart, doctor and uninstall use the receipt's target.
`host status --json` distinguishes invoking, installed and running identities;
doctor compares them by equality, including rollback divergence.

Removal deletes recorded integrations and validated package generations and
retains user data. Changed files and unowned development commands cause an error
or remain outside the removal set. Legacy installations without receipts must
be identified before adoption; discovery on PATH alone never establishes ownership.
Shared WASI caches from older installations are retained.

## Transaction

Installers lock the channel, stage beside the destination, validate all files
and run the packaged Calculator probe. A durable journal preserves the previous
receipt and integrations before activation. File activation uses atomic writes
on the destination filesystem. The installed target must pass the runtime probe
before the journal is cleared. An interrupted transaction recovers on the next
installer or launcher invocation; prior generations remain available.

`plexi update --rollback` uses the same activation path for the retained previous
package. Foreground and background updates call the distribution library
directly; no downloaded shell pipeline can report a false success. Restart
waits for the old process to exit before launching the recorded active generation.

## Release selection and publication

`release::accepts` is the shared policy: stable accepts stable, beta accepts beta
and stable, alpha accepts alpha, beta and stable. Selection paginates releases,
orders valid tags semantically, excludes drafts, and requires both the correct
channel/platform archive and checksum. The installation channel stays fixed.

Required platforms are macOS Apple Silicon, Linux x64 and Windows x64. macOS
Intel is optional; publication waits for its job and includes its complete
result if successful. Archive names come from `release::archive_name` and
include a channel suffix for alpha and beta. Each archive and native installer
has a matching `.sha256` sidecar.

`distribution.yml` runs host tests, lint/build gates, native transaction fixtures
and real package installation/GUI checks for each consumer channel. Publication
requires that workflow, checks the exact required artifact set, creates a draft,
attaches verified assets and only then publishes. Published tags and assets are
immutable. Retry a failed draft only after inspecting its existing assets;
publication never clobbers them.

SDK publication and the agent-skill mirror are ancillary to desktop availability.
Their separate workflows do not substitute for package gates. Signing credentials,
notarization and a fresh-machine Gatekeeper/SmartScreen pass remain release-owner
responsibilities; fixture tests do not establish those results.

## Validation

`test-distribution.py` uses tiny host fixtures to exercise ownership, recovery,
rollback, simultaneous installs, PATH integration, checksums and bootstrap HTTP
failures. `test-package.py` runs the real packaged WASI guest and optionally
launches a native host, drives terminal I/O and Calculator, captures through
`host screenshot`, restarts and removes the channel. Inspect the resulting PNG.
`PLEXI_DISTRIBUTION_HOME` isolates the installation registry for disposable test
runs; it is separate from the package's persisted installation root.

Linux support is x86_64 under X11, with Ubuntu 22.04 as the package build floor.
The host depends on the system X11, ALSA and graphics libraries installed by
`.github/actions/plexi-ci-env/action.yml`; GUI validation additionally uses an
X display. Wayland, Linux hardware video decoding and encrypted Linux secret
storage are outside this distribution contract.
