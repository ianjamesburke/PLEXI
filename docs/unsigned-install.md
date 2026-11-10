# Opening a Plexi build

Public Mac releases are signed with the self-signed `Plexi Dev` identity and
are not notarized. macOS Gatekeeper can warn on first open because the identity
is not an Apple Developer ID. Windows builds lack Authenticode signing. Only bypass that warning
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

## Windows

When SmartScreen displays **Windows protected your PC**:

1. Confirm the ZIP or installer came from an [official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).
2. Select **More info**.
3. Check that the app name and source are the release you downloaded, then select **Run anyway**.

Do not choose **Run anyway** for a build from an untrusted mirror, attachment,
or link.

## Linux

If you unpacked the archive yourself and the `plexi` file will not run, make it
executable:

```sh
chmod +x plexi
./plexi
```

The Plexi installer sets this permission automatically.
