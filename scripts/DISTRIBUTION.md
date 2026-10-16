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

The bootstrap pins the installer and its checksum to one released tag.
`PLEXI_BOOTSTRAP_TAG` overrides selection. Otherwise `--channel` / `-Channel`
`alpha` and `beta` follow `release::select` (`crates/distribution/src/release.rs`):
alpha prefers the newest alpha tag, then beta, then stable, and beta takes the
greatest SemVer among beta and stable. The default and `stable` use the latest
stable release. Neither public entrypoint downloads executable code from
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
`PLEXI_SIGN_IDENTITY` selects an available signing identity; local builds may
fall back to ad-hoc signing. A public Alpha build may not: release CI imports
the protected persistent `Plexi Dev` PKCS#12 identity and compares its leaf
certificate SHA-1 against `PLEXI_ALPHA_SIGNING_CERT_SHA1`. Package assembly
records the authority, certificate digest, and designated requirement; CI
re-extracts the published archive and verifies those facts before publication.
Replacing that identity is a security migration, not routine certificate
rotation: it invalidates the Keychain ACL that protects sealed grants and
Needs-you journals across updates.

Public beta and stable builds are deliberately refused until a separate
Developer ID plus notarization implementation supplies its own credentials and
clean-machine acceptance. Windows Authenticode remains outside the Alpha
contract. Broad nontechnical distribution still needs those credentials and OS
trust validation.

Release-owner setup: retain the existing `Plexi Dev` certificate and private
key together in a password-protected PKCS#12 backup. Configure repository Actions
secrets `PLEXI_ALPHA_SIGNING_P12_BASE64` (base64 of that backup),
`PLEXI_ALPHA_SIGNING_P12_PASSWORD`, and `PLEXI_ALPHA_SIGNING_CERT_SHA1` (the
certificate fingerprint, not its name). Never generate a replacement identity
in CI. Missing credentials or a fingerprint mismatch stop publication. The
imported key is kept in a temporary runner Keychain, selected explicitly for
signing without replacing the default Keychain, and removed
at job teardown. The runner trusts only the pinned certificate for code signing;
disposal of the hosted VM discards its temporary admin-domain trust setting.
Cleanup unlinks the temporary Keychain files rather than calling macOS trust
removal APIs, which can block on authorization even during cancellation.
The temporary Keychain is prepended to the runner's search list for codesign's
internal identity lookup; cleanup restores the saved original list.

`scripts/test-macos-keychain-continuity.py` installs distinct A and B packages
into temporary destinations, files a question under A, and requires the same
sealed Needs-you record under B without key recreation, rotation, or recovery.
It uses a disposable private Keychain and checks that the user's default
Keychain and search list are unchanged. CI rebuilds B with a different compiled
build tag; identical executable bytes are rejected as insufficient evidence.

For CI qualification without publication, dispatch `release.yml` with
`verify_only=true` and `tag` set to the source ref to test. This runs the real
package/signing acceptance with a synthetic Alpha build tag, then skips the
publish job. It creates neither a Git tag nor a GitHub Release.
For a fast credential check, use `signing_only=true` instead: it imports the
same identity, signs a disposable executable, verifies the fingerprint and
designated requirement, and removes the runner's temporary Keychain files.
Signing setup runs before compilation with a bounded timeout. The disposable
runner installs code-signing trust in its admin domain through noninteractive
sudo so it cannot wait on a user-domain authorization dialog.

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
cannot identify an executable. After a successful install the installer writes
that marker in the channel profile directory. The file contains the package
tag, or the build id when the tag is empty. SDK and core-app refresh stamps use the compiled
build identity, including source changes between prerelease builds.

Bare `plexi` delegates to the pane's named channel only when `PLEXI_RUNNING=1`.
Outside a pane it launches stable. Channel-named commands retain their channel.
Unix installs keep generated zsh, bash and fish completions under the receipt
root and register loading in the user's shell configuration. Bash is registered
in `~/.bashrc` and on the login file a login interactive bash reads
(`.bash_profile`, else `.bash_login`, else `.profile`). The `.profile` copy
runs only when `BASH_VERSION` is set. Host panes exec `bash -i -l`, which does
not read `~/.bashrc`. Zsh registration
loads the definition even when an existing completion cache references a missing
legacy file. Completion files participate in activation and rollback; removal
deletes only that channel's registration and files. PR installs skip completions.
`host start`, update, restart, doctor and uninstall use the receipt's target.
`host status --json` distinguishes invoking, installed and running identities;
doctor compares them by equality, including rollback divergence.

Removal deletes recorded integrations and validated package generations and
retains user data by default. Interactive uninstall offers a data-deletion choice;
`uninstall --delete-data --yes` explicitly deletes the selected channel's profile
and its permission seal, audit tip and Needs you journal tip from the OS credential
store. `--keep-data` skips that choice. The flags are mutually exclusive.
Other channels, workspace files outside the profile, shared credentials and the
shared host signing key remain intact. Credential cleanup runs before profile
removal, and a cleanup failure is reported without removing the installation so
it can be retried. Changed files and unowned development commands cause an error
or remain outside the removal set. Legacy installations without receipts must
be identified before adoption; discovery on PATH alone never establishes ownership.
Shared WASI caches from older installations are retained. A legacy release's
duplicated Unix command can be adopted only when its bytes match the payload at
the exact historical channel path; the legacy payload itself is retained.
Repository channel-list, cleanup and smoke tools query the native installer's
`list`/`locate` commands, including installations at custom destinations.

Host startup restores an absent channel-neutral agent-hook executable before
creating panes. Existing scripts and agent configurations are preserved; hook
registration remains explicit through `agent hook install`.

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
and stable, alpha accepts alpha, beta and stable. Alpha automatic selection gives
alpha prereleases priority, then beta prereleases, before stable fallback releases;
this prevents a same-series stable tag from demoting an installed alpha prerelease.
An explicit tag remains exact. Selection paginates releases, orders candidates
semantically within that policy, excludes drafts, and requires both the correct
channel/platform archive and checksum. The installation channel stays fixed.

Alpha releases (`vX.Y.Z-alpha.N`) support only macOS Apple Silicon (ARM64).
They build and validate only that platform's Alpha package and native installer.
This keeps the daily dogfood release loop focused on the platform in active use,
without waiting for builds and failures on platforms that are not actively used.

The same rule applies to pull-request and alpha-push CI: `distribution.yml` runs only the
Apple Silicon job unless the `tag` input is a stable or beta release tag, and `build-windows.yml`
runs only on `feature/windows-**` pushes and on demand, not on every PR.
The package job's host test + clippy step runs only for release tags; on PRs and alpha pushes
`rust-host.yml` already runs it.

Beta and stable releases require macOS Apple Silicon, Linux x64 and Windows x64.
macOS Intel is optional for those releases; publication waits for its job and
includes its complete result if successful. Archive names come from `release::archive_name` and
include a channel suffix for alpha and beta. Each archive and native installer
has a matching `.sha256` sidecar.

`distribution.yml` runs host tests, lint/build gates, native transaction fixtures
and real package installation/GUI checks for Alpha only on Alpha releases, and
for every consumer channel on Beta/stable releases and pull requests. Publication
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
failures. `test-install-bootstrap.sh` checks the public bootstrap's resolved tag
for each channel against a fake release API (`PLEXI_RELEASES_URL`). `test-package.py` runs the real packaged WASI guest and optionally
launches a native host, drives terminal I/O and Calculator, captures through
`host screenshot`, restarts and removes the channel. Inspect the resulting PNG.
`test-uninstall-data.py --package <alpha-package> --installer <native-installer>`
checks macOS keep, cancel, delete and reinstall flows in a temporary home and
private Keychain, including other-profile and shared-key preservation.
`PLEXI_DISTRIBUTION_HOME` isolates the installation registry for disposable test
runs; it is separate from the package's persisted installation root.
Windows GUI tests require a disposable OS user, as Windows Known Folders and
user PATH registration remain account-scoped despite HOME environment overrides.

Linux support is x86_64 under X11, with Ubuntu 22.04 as the package build floor.
The host depends on the system X11, ALSA and graphics libraries installed by
`.github/actions/plexi-ci-env/action.yml`; GUI validation additionally uses an
X display. Wayland, Linux hardware video decoding and encrypted Linux secret
storage are outside this distribution contract.
