# Windows installer

A traditional per-user Windows installer wizard built with [Inno Setup 6](https://jrsoftware.org/isinfo.php).

The output is a single double-click `Setup.exe` that:

- Copies `plexi.exe` to `%LOCALAPPDATA%\Plexi\bin\`
- (Optional task) Adds that directory to the user PATH
- (Optional task) Registers PowerShell tab-completions in `$PROFILE.CurrentUserAllHosts`
- Registers an Add/Remove Programs entry with a working Uninstall.exe

No Administrator rights required — everything lives under HKCU and the user profile.

## Build

1. Install Inno Setup 6 (one-time):
   - `winget install JRSoftware.InnoSetup` (per-user, installs to `%LOCALAPPDATA%\Programs\Inno Setup 6\`), or
   - Direct download from https://jrsoftware.org/isdl.php (system-wide, installs to `Program Files (x86)\Inno Setup 6\`).
2. Build the release binary:
   ```powershell
   cargo build --release --bin plexi
   ```
3. Compile the installer (pass the version via `/DMyAppVersion=` so the `.iss` doesn't have to parse `Cargo.toml`):
   ```powershell
   # Locate ISCC regardless of install scope.
   $iscc = @(
     "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
     "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
     "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
   ) | Where-Object { Test-Path $_ } | Select-Object -First 1

   # Extract version from Cargo.toml.
   $ver = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

   # Stable channel -> dist\plexi-setup.exe
   & $iscc /DMyAppVersion=$ver installer\plexi.iss

   # Alpha channel  -> dist\plexi-alpha-setup.exe
   & $iscc /DMyAppVersion=$ver /DMyChannel=alpha installer\plexi.iss

   # Beta channel   -> dist\plexi-beta-setup.exe
   & $iscc /DMyAppVersion=$ver /DMyChannel=beta  installer\plexi.iss
   ```

Output lands in `dist\` at the repo root. PR builds are not packaged as installers — use `scripts\install-windows.ps1 -Channel pr-<N>` directly.

## What the installer does (matches scripts\install-windows.ps1)

| Step                       | Stable                                 | Alpha (example)                            |
| -------------------------- | -------------------------------------- | ------------------------------------------ |
| Install dir                | `%LOCALAPPDATA%\Plexi\`                | `%LOCALAPPDATA%\Plexi-alpha\`              |
| Binary name                | `bin\plexi.exe`                        | `bin\plexi-alpha.exe`                      |
| PATH segment (added)       | `%LOCALAPPDATA%\Plexi\bin`             | `%LOCALAPPDATA%\Plexi-alpha\bin`           |
| `$PROFILE` sentinel        | `# >>> plexi completions (plexi.exe)`  | `# >>> plexi completions (plexi-alpha.exe)`|
| Add/Remove Programs entry  | `Plexi <version>`                      | `Plexi Alpha <version>`                    |

Different channels have distinct `AppId` GUIDs, install dirs, and sentinel tags — they coexist side-by-side and uninstall independently.

## Uninstalling

Three equivalent paths:

- **GUI**: Settings -> Apps -> Plexi[/Alpha/Beta] -> Uninstall
- **Wizard**: run `unins000.exe` inside the install dir
- **Script**: `scripts\uninstall-windows.ps1` (also works for installs done via `install-windows.ps1`)

All three reverse the same things: binary, PATH segment, `$PROFILE` block. Two separate trees are involved:

- **Install dir** (`%LOCALAPPDATA%\Plexi[-channel]\`) — the binary and bundled Python runtime. Removed by the GUI/wizard uninstaller.
- **Profile dir** (`~\.plexi[-channel]\`) — config, logs, installed apps, and secrets. **Kept by default**, and NOT touched by the GUI/wizard uninstaller (Inno only manages its own install dir).

To remove the profile dir as well, run `scripts\uninstall-windows.ps1 -PurgeProfile` (it removes both the install dir and the profile dir).

## Icon

If `assets\app-icon.ico` exists, the installer uses it for the Setup.exe icon and the Add/Remove Programs entry. The `.icns` and `.png` masters live in `assets\` already; generating the `.ico` is a one-time conversion (`magick assets\app-icon.png -define icon:auto-resize=16,24,32,48,64,128,256 assets\app-icon.ico` or equivalent). If the `.ico` is missing the installer still builds — it just uses the default Inno icon.
