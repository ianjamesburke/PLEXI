# Plexi v1 binary install

Plexi v1 consumer installs download release assets. They do not require Homebrew,
apt, yay, winget, Git, or a Rust toolchain.

## Assets

Every GitHub release publishes these archives:

| Platform | Asset | Contents |
| --- | --- | --- |
| macOS Apple Silicon | `plexi-macos-arm64.tar.gz` | `Plexi.app` and `plexi` |
| macOS Intel | `plexi-macos-x64.tar.gz` | `Plexi.app` and `plexi` |
| Linux x86_64 | `plexi-linux-x64.tar.gz` | `plexi` |
| Windows x64 | `plexi-windows-x64.zip` | `plexi.exe` |

The public command is `curl -fsSL https://plexiapp.com/install | sh`. It installs
under `~/.local/share/plexi/<channel>` and puts the channel command in
`~/.local/bin`: `plexi`, `plexi-beta`, or `plexi-alpha`. Set `PLEXI_INSTALL_DIR`
or `PLEXI_BIN_DIR` before running it to choose user-owned destinations.

Until this branch lands on `main`, the served top-level installer still delegates
to `main/scripts/install.sh`. To exercise the binary installer before that merge,
download this branch's `scripts/install.sh` directly (or run it from this
checkout); it is the version that selects and downloads release assets.

`scripts/install.sh --dry-run --channel alpha` prints the selected platform,
asset URL, and destinations without downloading. CI can point
`PLEXI_RELEASE_BASE_URL` at a fixture server to exercise archive extraction.

## Channels

Stable uses ordinary tags (`vX.Y.Z`) and the `plexi` command. Beta uses
`vX.Y.Z-beta.N` and `plexi-beta`; alpha uses `vX.Y.Z-alpha.N` and `plexi-alpha`.
Install a non-stable channel with `--channel beta` or `--channel alpha`. Profiles
remain isolated as `~/.plexi/`, `~/.plexi-beta/`, and `~/.plexi-alpha/`.

`plexi update` follows the same channel policy. v1 tags must contain the matching
asset; releases made before this cutover are source-era releases and are not
auto-installed as binaries. Contributors can still build a checkout explicitly
with `scripts/install.sh --from-source [channel]`.

## Unsigned builds

v1 assets are deliberately unsigned and not notarized. On macOS, open a blocked
app with Finder's contextual **Open** command and confirm the dialog. On Windows,
use SmartScreen's **More info** then **Run anyway** only after confirming the
download came from the Plexi GitHub release. Linux may need `chmod +x plexi` when
an archive was unpacked manually; the installer sets it automatically.

## Release dry run

Run the GitHub Actions workflow manually with an existing tag. It builds workflow
artifacts first, then creates or updates that tag's GitHub release after macOS and
Linux succeed. Windows is currently `continue-on-error` while its platform seam
receives its first hosted validation, so it is reported but does not block those
assets. The `publish` job uses `gh release upload --clobber`, so rerunning it
replaces assets rather than accumulating stale copies.
