# Plexi v1 binary install

Plexi v1 consumer installs download release assets. They do not require Homebrew,
apt, yay, winget, Git, or a Rust toolchain.

On Linux, the installer is a self-contained user-directory install, not a distro
package: Plexi does not provide an apt, yay, AUR, Flatpak, AppImage, `.deb`, or
`.rpm` path. Linux support is currently x86_64 under X11; Wayland is not
supported. Linux secrets use a mode-`0600` file, not an encrypted OS keyring,
and Linux hardware video decoding is not implemented.

## Assets

Alpha releases publish these download archives when the corresponding platform
build is available:

| Platform | Asset | Contents |
| --- | --- | --- |
| macOS Apple Silicon | `plexi-macos-arm64.tar.gz` | `Plexi.app` and `plexi` |
| macOS Intel (when published) | `plexi-macos-x64.tar.gz` | `Plexi.app` and `plexi` |
| Linux x86_64 | `plexi-linux-x64.tar.gz` | `plexi` |
| Windows x64 | `plexi-windows-x64.zip` | `plexi.exe` |

Each archive has a matching `.sha256` sidecar. The installers verify it before
replacing an existing install; see [Opening an unsigned Plexi build](unsigned-install.md)
for the separate operating-system trust prompts.

## Failed installs

An invalid channel, unavailable asset, unavailable or mismatched checksum, or
invalid archive exits nonzero without printing an install-success line. Downloads
and extraction finish in a temporary directory before activation. If activation
is interrupted, the installer restores the previous payload, command, and
`installed_tag` marker (or removes any newly-created marker), so a prior working
install remains the reported install.

## Windows (x64)

Run this download installer in Windows PowerShell 5.1 or later (no Rust or
Visual Studio):

```powershell
irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/alpha/scripts/install-windows.ps1 | iex
```

It downloads the newest Windows x64 alpha release and verifies its checksum; it
does not build from source. The Unix `curl | bash` installer is not a Windows
install path. To pin a tag/channel:

```powershell
iex "& { $(irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/alpha/scripts/install-windows.ps1) } -Channel alpha -Tag vX.Y.Z-alpha.N"
```

Expect `plexi-alpha.exe --version` and `%USERPROFILE%\.plexi-alpha\installed_tag` matching the tag.
`plexi-alpha update` downloads the newer `plexi-windows-x64.zip` via the same script (asset-prefer, not cargo).


## macOS and Linux downloads

On macOS Apple Silicon and Linux x86_64, the public command is
`curl -fsSL https://plexiapp.com/install | bash`. It installs
under `~/.local/share/plexi/<channel>` and puts the channel command in
`~/.local/bin`: `plexi`, `plexi-beta`, or `plexi-alpha`. Set `PLEXI_INSTALL_DIR`
or `PLEXI_BIN_DIR` before running it to choose user-owned destinations.

The installer chooses the matching macOS architecture. Apple Silicon is the
current expected asset; Intel installs when the selected release includes
`plexi-macos-x64.tar.gz`, otherwise the installer reports that the asset is not
published. Linux requires an X11 session; Wayland is not supported. The
installer requires `bash`, `curl`, and `tar`, plus `shasum` on macOS or
`sha256sum` (or `shasum`) on Linux.

The served top-level installer delegates to `alpha/scripts/install.sh` and
defaults to `--channel alpha` until `main` publishes binary assets. Auto tag selection skips channel cuts that lack the platform asset (so an empty `alpha.N` waiting on Actions falls back to the newest cut that has it). An explicit
`--channel` still selects that channel; from a macOS or Linux checkout,
contributors can opt into a source build with `scripts/install.sh --from-source
alpha`. That path requires Rust and Python 3. On Windows, Git Bash receives a
PowerShell command instead of attempting a Unix install; Windows has no
source-install script.

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

v1 assets are deliberately unsigned and not notarized. See [Opening an unsigned
Plexi build](unsigned-install.md) for the Finder **Open** flow and optional
`xattr -cr` command on macOS, the SmartScreen **More info** → **Run anyway** flow
on Windows, and the manual Linux `chmod +x` fallback.

## Release dry run

Run the GitHub Actions workflow manually with an existing tag. It builds workflow
artifacts first, then creates or updates that tag's GitHub release after macOS and
Linux succeed. Windows is currently `continue-on-error` while its platform seam
receives its first hosted validation, so it is reported but does not block those
assets. The `publish` job uses `gh release upload --clobber`, so rerunning it
replaces assets rather than accumulating stale copies.
