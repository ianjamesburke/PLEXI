# Opening an unsigned Plexi build

Public stable, alpha and beta builds currently lack Developer ID notarization
and Windows Authenticode signing. Your operating system may show a
warning the first time you open a downloaded build. Only bypass that warning
when the archive came from an [official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).

## macOS

If macOS says it cannot verify Plexi, or that the plugin cannot be opened because
it is from an unidentified developer:

1. In Finder, find `Plexi.plugin` in your home directory's `Applications` folder.
2. Control-click or right-click `Plexi.plugin`, then choose **Open**.
3. In the confirmation dialog, choose **Open** again.

This approves that copy of Plexi. Opening the plugin normally afterward works as
expected.

If Finder still blocks the plugin, remove the download quarantine attribute in
Terminal. Adjust the path if you placed the plugin somewhere else:

```sh
xattr -cr "$HOME/Applications/Plexi.plugin"
open "$HOME/Applications/Plexi.plugin"
```

`xattr -cr` removes extended attributes from the plugin bundle. Run it only for a
Plexi plugin you downloaded from the official release page.

## Windows

When SmartScreen displays **Windows protected your PC**:

1. Confirm the ZIP or installer came from an [official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).
2. Select **More info**.
3. Check that the plugin name and source are the release you downloaded, then select **Run anyway**.

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
