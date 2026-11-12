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
just, uv and Python 3.11 or newer. Arguments after the optional channel pass
through to the installer, e.g. `scripts/install.sh --from-source beta --trash-strays`.

After activation the installer lists other copies of the channel's Mac app
outside the managed installation (`strays::find_strays`): `/Applications`,
`~/Applications`, `~/Downloads` and `~/Desktop`, matched by
`CFBundleIdentifier` or, without a readable identifier, by bundle name. The
Trash, the installation root and owned integrations are excluded. By default it
only warns. `--trash-strays`, or answering yes to the prompt shown when stdin
is a terminal, renames them into `~/.Trash` (numbered on collision); nothing
is deleted. The public bootstrap forwards the flag unchanged.

## Repository installer cache

`scripts/distribution-tool.sh` runs the authoritative native installer through a
small verified executable cache outside disposable worktrees. Each input identity has its own immutable executable path, so a later build cannot replace an installer already handed to packaging. The cache checks
installer source, lockfile, toolchain, platform, flags and binary digest. Warm
locate/remove calls do not invoke Cargo or its global lease. Missing or mismatched
evidence rebuilds the native installer under the existing lease; this cold path
is reported explicitly. `PLEXI_DISTRIBUTION_CACHE_ROOT` relocates this cache.
Source installation uses the same helper.

Cleanup accepts absence only from the native locator's exit code 2. Lookup,
ownership or self-uninstall failures preserve files and return failure. A missing managed host executable is a recovery case: stop that channel, obtain its receipt root from the native `list` output, then use `scripts/distribution-tool.sh remove --receipt <root>`; cleanup does not guess a root or bypass native ownership checks. The
native locator rejects a registry pointing to another channel. Bundles,
launchers, receipts and generations are removed by the owning native transaction,
never by guessing channel filenames. Explicit ephemeral profile cleanup remains
separate; standing-channel cleanup retains user data.

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
fall back to ad-hoc signing. A public release build may not: release CI imports
the protected persistent `Plexi Dev` PKCS#12 identity and compares its leaf
certificate SHA-1 against the pinned release signing secret. Package assembly
records the authority, certificate digest, and designated requirement; CI
re-extracts the published archive and verifies those facts before publication.
Replacing that identity is a security migration, not routine certificate
rotation: it invalidates the Keychain ACL that protects sealed grants and
Needs-you journals across updates.

Public Mac releases use the self-signed `Plexi Dev` identity and are not
notarized. See [the opening guide](../docs/unsigned-install.md) for the macOS
warning and first-open steps. Developer ID and notarization remain future work.

Release-owner setup: retain the existing `Plexi Dev` certificate and private
key together in a password-protected PKCS#12 backup. Store it as the
`release-signing` environment secrets `PLEXI_ALPHA_SIGNING_P12_BASE64` (base64 of that backup),
`PLEXI_ALPHA_SIGNING_P12_PASSWORD`, and `PLEXI_ALPHA_SIGNING_CERT_SHA1` (the
certificate fingerprint, not its name); the approval rules for that environment
are in [Release Channels](RELEASE_CHANNELS.md#signed-publish-approval). Never generate a replacement identity
in CI. Missing credentials or a fingerprint mismatch stop publication. The
imported key is kept in a temporary runner Keychain, selected explicitly for
signing without replacing the default Keychain, and removed
at job teardown. The runner trusts only the pinned certificate for code signing;
disposal of the hosted VM discards its temporary admin-domain trust setting.
Cleanup unlinks the temporary Keychain files rather than calling macOS trust
removal APIs, which can block on authorization even during cancellation.
The temporary Keychain is prepended to the runner's search list for codesign's
internal identity lookup; cleanup restores the saved original list.

For Alpha releases, `scripts/test-macos-keychain-continuity.py` installs distinct A and B packages
into temporary destinations, files a question under A, and requires the same
sealed Needs-you record under B without key recreation, rotation, or recovery.
It uses a disposable private Keychain and checks that the user's default
Keychain and search list are unchanged. CI rebuilds B with a different compiled
build tag; identical executable bytes are rejected as insufficient evidence.

For CI qualification without publication, dispatch `release.yml` with
`verify_only=true` and `tag` set to the source ref to test. This runs the real
package/signing acceptance with a synthetic Alpha build tag, then skips the
publish job. Add `verify_stable=true` to exercise the stable matrix and
all three consumer channels with a synthetic stable tag. It creates neither a Git tag nor a GitHub Release.
For a fast credential check, use `signing_only=true` instead: it imports the
same identity, signs a disposable executable, verifies the fingerprint and
designated requirement, and removes the runner's temporary Keychain files.
Signing setup runs before compilation with a bounded timeout. The disposable
runner installs code-signing trust in its admin domain through noninteractive
sudo so it cannot wait on a user-domain authorization dialog.

## Layout and ownership

macOS uses per-user `~/Applications/Plexi.app`, `Plexi Alpha.app` and
`Plexi Beta.app` bundles copied from the active immutable generation. Linux uses an XDG data
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

The Applications bundle is a real directory, not a link: Spotlight indexes
neither symlinks nor hidden directories, so a link into
`~/.local/share/plexi/<channel>/generations/<hash>/` was unsearchable, and
LaunchServices kept one record per generation path for `open -a` to choose
from. Activation copies the generation's bundle with `ditto` (preserving the
signature and extended attributes) into a hidden sibling scratch directory,
then renames the old item aside and the copy into place under the transaction
journal. After commit it runs `lsregister -f` on the copy and `lsregister -u` on
the bundles inside the generation store; failures are logged, not fatal. Only
our old link into this installation root or a copy the receipt records is
replaced; anything else is refused. The receipt records the copy's main
executable digest rather than a marker file, because a file inside `Contents/`
would break the code-signature seal. A copy lacks `package.json`, so the host
locates its package through the receipt by build id. Uninstall unregisters and
removes the owned copy. Command shims and receipt launches still execute the
generation directly.

The UI version label, `--version`, the menu title and `host status` show the
running binary's compiled tag (or crate version) and short commit. At launch
the host compares its build id with the channel receipt's active build, logs a
warning and posts one notification naming the relaunch path when they differ.
On macOS it also notifies about strays in `/Applications` and `~/Applications`;
Downloads and Desktop are left to the installer because reading them from the
app triggers privacy prompts. Launch never moves anything.

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

All releases require macOS Apple Silicon. Alpha releases also attempt Linux x64.
Beta and stable releases attempt Linux x64, Windows x64 and Intel Mac; each is
optional. The publisher logs each omitted optional platform and publishes a
platform only when its complete asset set and checksums pass. Archive names come
from `release::archive_name`, with channel suffixes for alpha and beta.

Pull requests use the Alpha matrix. `build-windows.yml` runs on Windows feature
branches and on demand. Package jobs run host tests and Clippy for release tags.

`distribution.yml` runs host tests, lint/build gates, native transaction fixtures
and real package installation/GUI checks for Alpha on Alpha releases, and
for every consumer channel on Beta/stable releases. Publication
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
X display. Wayland and Linux hardware video decoding are outside this
distribution contract. Linux secrets use Secret Service or the encrypted-file
fallback; plaintext profile storage is not supported.
