# v1 release artifacts

Plexi v1 releases ship downloadable, per-operating-system binaries through the
GitHub release for the channel tag. A release is not a source-only install
workflow.

## Required assets

Every v1 release requires these archives and their matching `.sha256` sidecars:

| Platform | Required archive |
| --- | --- |
| Linux x64 | `plexi-linux-x64.tar.gz` |
| macOS Apple Silicon | `plexi-macos-arm64.tar.gz` |
| Windows x64 | `plexi-windows-x64.zip` |

`plexi-macos-x64.tar.gz` is optional. It is a soft/best-effort artifact, so
release pages and installation guidance must not promise macOS Intel support
until that archive is published.

## Integrity and trust

The release workflow publishes a SHA-256 sidecar for every archive. The Unix
and Windows installers download both files, validate the archive hash before
unpacking it, and refuse installation on a missing, malformed, or mismatched
checksum. They stage downloads before replacing an existing installation, so a
failed integrity check preserves the prior install.

Alpha binaries are deliberately unsigned and not notarized. A successful
checksum check verifies the downloaded archive against the release sidecar; it
does not remove macOS Gatekeeper or Windows SmartScreen prompts. Follow
[Opening an unsigned Plexi build](unsigned-install.md) for those OS-specific
steps.

## Channels

Stable releases use `vX.Y.Z`, beta releases use `vX.Y.Z-beta.N`, and alpha
releases use `vX.Y.Z-alpha.N`. Installers and `plexi update` select release
assets accepted by the running channel. The detailed channel and promotion
rules live in [Release Channels](../scripts/RELEASE_CHANNELS.md); consumer
installation commands live in [v1 binary install](v1-binary-install.md). On
Windows, the PowerShell installer names those channels `stable`, `beta`, and
`alpha`: their commands are `plexi.exe`, `plexi-beta.exe`, and
`plexi-alpha.exe`, respectively.
