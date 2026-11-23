# Opening a Plexi build

Public Mac releases are signed with the self-signed `Plexi Dev` identity and
are not notarized. macOS Gatekeeper can warn on first open because the identity
is not an Apple Developer ID. Only bypass that warning
when the archive came from an [official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).

## macOS

If macOS blocks the app as an unidentified developer:

1. Open `Plexi.app` from your home directory's `Applications` folder once,
   then dismiss the warning.
2. Open **System Settings > Privacy & Security**. Under Security, choose
   **Open Anyway** for Plexi.
3. Confirm **Open Anyway** and authenticate if macOS asks.

On macOS versions that offer it, Control-click the app in Finder, choose
**Open**, and confirm. macOS remembers the approval for that app.

Only approve a copy from the official release linked above.
