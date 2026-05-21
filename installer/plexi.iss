; ============================================================================
; Plexi for Windows — Inno Setup installer
; ============================================================================
;
; Build (requires Inno Setup 6 — https://jrsoftware.org/isinfo.php):
;
;   Stable channel:
;     ISCC.exe installer\plexi.iss
;
;   Alpha / beta channel (binary installed as plexi-alpha.exe etc., own
;   profile dir, side-by-side with stable):
;     ISCC.exe /DMyChannel=alpha installer\plexi.iss
;     ISCC.exe /DMyChannel=beta  installer\plexi.iss
;
; Source binary defaults to ..\target\release\plexi.exe relative to this file.
; Override with /DSourceExe=...
;
; Output:
;   dist\plexi-setup.exe                (stable)
;   dist\plexi-alpha-setup.exe          (channel builds)
;
; Per-user install (PrivilegesRequired=lowest). Admin not required. Mirrors
; scripts\install-windows.ps1: same install root, same PATH segment, same
; sentinel-tagged completions block in $PROFILE.
; ============================================================================

#ifndef MyChannel
  #define MyChannel ""
#endif

#ifndef SourceExe
  #define SourceExe "..\target\release\plexi.exe"
#endif

#if MyChannel == ""
  #define MyAppNameSuffix ""
  #define MyBinaryName "plexi.exe"
  #define MyProfileDirName "Plexi"
  #define MyAppId "{B71F3AAE-3FF0-4F89-9F71-2A21E1EC0000}"
  #define MyOutputBase "plexi-setup"
#elif MyChannel == "alpha"
  #define MyAppNameSuffix " Alpha"
  #define MyBinaryName "plexi-alpha.exe"
  #define MyProfileDirName "Plexi-alpha"
  #define MyAppId "{B71F3AAE-3FF0-4F89-9F71-2A21E1EC0A1A}"
  #define MyOutputBase "plexi-alpha-setup"
#elif MyChannel == "beta"
  #define MyAppNameSuffix " Beta"
  #define MyBinaryName "plexi-beta.exe"
  #define MyProfileDirName "Plexi-beta"
  #define MyAppId "{B71F3AAE-3FF0-4F89-9F71-2A21E1EC0B1B}"
  #define MyOutputBase "plexi-beta-setup"
#else
  #error Unknown MyChannel value. Supported: (empty), alpha, beta. PR builds use install-windows.ps1.
#endif

; Version is supplied by the build wrapper via /DMyAppVersion=x.y.z so the .iss
; doesn't have to parse Cargo.toml. Falls back to 0.0.0 if omitted so the
; installer still compiles for ad-hoc local builds.
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif

#define MyAppPublisher "Plexi"
#define MyAppURL "https://github.com/zachristmas/PLEXI"

; Single source of truth for whether the icon asset is available, used to
; gate SetupIconFile, the bundled [Files] entry, [Icons] IconFilename, and
; UninstallDisplayIcon. Path is relative to the .iss file.
#define HasIcon FileExists(AddBackslash(SourcePath) + "..\assets\app-icon.ico")

[Setup]
AppId={{#MyAppId}}
AppName=Plexi{#MyAppNameSuffix}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={localappdata}\{#MyProfileDirName}
DefaultGroupName=Plexi{#MyAppNameSuffix}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
Compression=lzma2
SolidCompression=yes
OutputDir=..\dist
OutputBaseFilename={#MyOutputBase}
UninstallDisplayName=Plexi{#MyAppNameSuffix} {#MyAppVersion}
WizardStyle=modern
ChangesEnvironment=yes
#if HasIcon
SetupIconFile=..\assets\app-icon.ico
UninstallDisplayIcon={app}\app-icon.ico
#else
UninstallDisplayIcon={app}\bin\{#MyBinaryName}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath";   Description: "Add Plexi to your PATH (recommended)";                       GroupDescription: "Shell integration:"; Flags: checkedonce
Name: "completions"; Description: "Register PowerShell tab-completions in your $PROFILE";       GroupDescription: "Shell integration:"; Flags: checkedonce

[Files]
Source: "{#SourceExe}";                       DestDir: "{app}\bin"; DestName: "{#MyBinaryName}"; Flags: ignoreversion
Source: "plexi-completions-helper.ps1";       DestDir: "{app}";                                  Flags: ignoreversion
#if HasIcon
Source: "..\assets\app-icon.ico";             DestDir: "{app}";                                  Flags: ignoreversion
#endif

[Icons]
; WorkingDir defaults to {app} (the install dir) if omitted, which makes Plexi
; spawn new terminal panes inside %LOCALAPPDATA%\Plexi[-channel]\. Setting it
; to {userprofile} (~) gives terminals a sane default CWD when launched via
; the Start Menu shortcut. Terminal launches of `plexi` still honor the
; caller's CWD.
#if HasIcon
Name: "{group}\Plexi{#MyAppNameSuffix}"; Filename: "{app}\bin\{#MyBinaryName}"; WorkingDir: "{userprofile}"; IconFilename: "{app}\app-icon.ico"
#else
Name: "{group}\Plexi{#MyAppNameSuffix}"; Filename: "{app}\bin\{#MyBinaryName}"; WorkingDir: "{userprofile}"
#endif

[Run]
; Absolute powershell.exe path avoids relying on PATH order at install time.
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\plexi-completions-helper.ps1"" -Action register -BinaryName ""{#MyBinaryName}"" -BinaryPath ""{app}\bin\{#MyBinaryName}"""; \
  Tasks: completions; \
  StatusMsg: "Registering PowerShell completions..."; \
  Flags: runhidden

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\plexi-completions-helper.ps1"" -Action unregister -BinaryName ""{#MyBinaryName}"" -BinaryPath ""{app}\bin\{#MyBinaryName}"""; \
  Flags: runhidden

[Code]
const
  EnvironmentKey = 'Environment';

function GetUserPath(): string;
var
  Value: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Value) then
    Value := '';
  Result := Value;
end;

procedure SetUserPath(const NewPath: string);
begin
  if NewPath = '' then
    RegDeleteValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path')
  else
    RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', NewPath);
end;

function PathContainsSegment(const FullPath, Segment: string): Boolean;
var
  Padded: string;
begin
  Padded := ';' + LowerCase(FullPath) + ';';
  Result := Pos(';' + LowerCase(Segment) + ';', Padded) > 0;
end;

procedure AddPathSegment(const Segment: string);
var
  CurrentPath: string;
begin
  CurrentPath := GetUserPath();
  if PathContainsSegment(CurrentPath, Segment) then
    Exit;
  if (CurrentPath <> '') and (Copy(CurrentPath, Length(CurrentPath), 1) <> ';') then
    CurrentPath := CurrentPath + ';';
  SetUserPath(CurrentPath + Segment);
end;

procedure RemovePathSegment(const Segment: string);
var
  CurrentPath, NewPath, Item: string;
  SepPos: Integer;
  LowerSeg: string;
begin
  CurrentPath := GetUserPath();
  if CurrentPath = '' then
    Exit;
  LowerSeg := LowerCase(Segment);
  NewPath := '';
  while Length(CurrentPath) > 0 do
  begin
    SepPos := Pos(';', CurrentPath);
    if SepPos = 0 then
    begin
      Item := CurrentPath;
      CurrentPath := '';
    end else begin
      Item := Copy(CurrentPath, 1, SepPos - 1);
      CurrentPath := Copy(CurrentPath, SepPos + 1, Length(CurrentPath));
    end;
    if (Item <> '') and (LowerCase(Item) <> LowerSeg) then
    begin
      if NewPath = '' then
        NewPath := Item
      else
        NewPath := NewPath + ';' + Item;
    end;
  end;
  SetUserPath(NewPath);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddPathSegment(ExpandConstant('{app}\bin'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemovePathSegment(ExpandConstant('{app}\bin'));
end;
