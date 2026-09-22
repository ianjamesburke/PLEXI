# Plexi v1 binary install

Plexi v1 consumer installs download release assets. They do not require Homebrew,
apt, yay, winget, Git, or a Rust toolchain.

On Linux, the installer is a self-contained user-directory install, not a distro
package: Plexi does not provide an apt, yay, AUR, Flatpak, AppImage, `.deb`, or
`.rpm` path. Linux support is currently x86_64 under X11; Wayland is not
supported. Linux secrets use a mode-`0600` file, not an encrypted OS keyring,
and Linux hardware video decoding is not implemented.

## Assets

Every GitHub release publishes these archives:

| Platform | Asset | Contents |
| --- | --- | --- |
| macOS Apple Silicon | `plexi-macos-arm64.tar.gz` | `Plexi.app` and `plexi` |
| macOS Intel | `plexi-macos-x64.tar.gz` | `Plexi.app` and `plexi` |
| Linux x86_64 | `plexi-linux-x64.tar.gz` | `plexi` |
| Windows x64 | `plexi-windows-x64.zip` | `plexi.exe` |

Each archive has a matching `.sha256` sidecar. The installers verify it before
replacing an existing install; see [Opening an unsigned Plexi build](unsigned-install.md)
for the separate operating-system trust prompts.

## Windows (x64)

Preferred one-liner (no Rust):

```powershell
irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/v0.3.1-windows.1/scripts/install-windows.ps1 | iex
```

Or pin a tag/channel:

```powershell
iex "& { $(irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/v0.3.1-windows.1/scripts/install-windows.ps1) } -Channel alpha -Tag v0.3.1-windows.1"
```

Expect `plexi-alpha.exe --version` and `%USERPROFILE%\.plexi-alpha\installed_tag` matching the tag.
`plexi-alpha update` downloads the newer `plexi-windows-x64.zip` via the same script (asset-prefer, not cargo).


The public command is `curl -fsSL https://plexiapp.com/install | sh`. It installs
under `~/.local/share/plexi/<channel>` and puts the channel command in
`~/.local/bin`: `plexi`, `plexi-beta`, or `plexi-alpha`. Set `PLEXI_INSTALL_DIR`
or `PLEXI_BIN_DIR` before running it to choose user-owned destinations.

The served top-level installer delegates to `alpha/scripts/install.sh` and
defaults to `--channel alpha` until `main` publishes binary assets. Auto tag selection skips channel cuts that lack the platform asset (so an empty `alpha.N` waiting on Actions falls back to the newest cut that has it). An explicit
`--channel` still selects that channel; contributors can opt into a source build
with `--from-source`.

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
