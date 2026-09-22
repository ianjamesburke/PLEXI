# Opening an unsigned Plexi build

Plexi alpha builds are unsigned on purpose. Your operating system may show a
warning the first time you open a downloaded build. Only bypass that warning
when the archive came from an [official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).

## macOS

If macOS says it cannot verify Plexi, or that the app cannot be opened because
it is from an unidentified developer:

1. In Finder, find `Plexi.app` (usually in `Applications` after installation).
2. Control-click or right-click `Plexi.app`, then choose **Open**.
3. In the confirmation dialog, choose **Open** again.

This approves that copy of Plexi. Opening the app normally afterward works as
expected.

If Finder still blocks the app, remove the download quarantine attribute in
Terminal. Adjust the path if you placed the app somewhere else:

```sh
xattr -cr /Applications/Plexi.app
open /Applications/Plexi.app
```

`xattr -cr` removes extended attributes from the app bundle. Run it only for a
Plexi app you downloaded from the official release page.

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
