; NeoAmp's Windows installer, made with Inno Setup 6 (https://jrsoftware.org/isinfo.php).
; windows\package.ps1 builds neoamp.exe and compiles this script with ISCC, giving it:
;
;   /DAppVersion=<version>   Cargo.toml's version
;   /DBinary=<path>          the neoamp.exe to install
;   /DOutputDir=<folder>     where to write neoamp-v<version>-windows-x86_64-setup.exe
;
; It installs for the current user (no administrator needed) unless the person installing
; picks all users in the first dialog, or passes /ALLUSERS. Installing a later version over
; it, in either mode, replaces it in place, closing a running NeoAmp first.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>: windows\package.ps1 does
#endif
#ifndef Binary
  #error Pass /DBinary=<path to neoamp.exe>: windows\package.ps1 does
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
; Identifies NeoAmp to every version of its installer, and in Apps & features: never change it.
AppId={{092209C5-E961-48D1-BC1C-68D0CCBB01DD}
AppName=NeoAmp
AppVersion={#AppVersion}
AppPublisher=Blake Gardner
AppPublisherURL=https://github.com/BlakeGardner/NeoAmp
AppSupportURL=https://github.com/BlakeGardner/NeoAmp/issues
AppUpdatesURL=https://github.com/BlakeGardner/NeoAmp/releases
AppComments=Play music and internet radio with classic Winamp skins
; {autopf} is %LOCALAPPDATA%\Programs for the current user, Program Files for all users.
DefaultDirName={autopf}\NeoAmp
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; A 64-bit program, which Windows 11 on Arm runs too.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10, the oldest that Rust's programs run on.
MinVersion=10.0
; A running NeoAmp is closed so its program can be replaced. It saves its settings within
; seconds of a change, so closing it loses next to nothing.
CloseApplications=force
; Tells Explorer about the .wsz association below.
ChangesAssociations=yes
UninstallDisplayName=NeoAmp
UninstallDisplayIcon={app}\neoamp.exe
SetupIconFile=..\resources\icons\neoamp.ico
WizardStyle=modern
OutputDir={#OutputDir}
OutputBaseFilename=neoamp-v{#AppVersion}-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Binary}"; DestDir: "{app}"; DestName: "neoamp.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\NeoAmp"; Filename: "{app}\neoamp.exe"; Comment: "Play music and internet radio with classic Winamp skins"
Name: "{autodesktop}\NeoAmp"; Filename: "{app}\neoamp.exe"; Tasks: desktopicon

[Registry]
; Classic skins (.wsz) open in NeoAmp: listed under Open with, and opened on a double-click
; unless another program already opens them (Winamp, say), which keeps them. A skin opened
; while NeoAmp runs goes to the running NeoAmp. HKA is the current user's keys, or the
; machine's when installing for all users.
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz"; ValueType: string; ValueName: ""; ValueData: "Winamp classic skin"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
; Uninstalling goes through these in reverse, so the .wsz key, if NeoAmp leaves it empty, is
; removed after its OpenWithProgids.
Root: HKA; Subkey: "Software\Classes\.wsz"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.wsz"; Flags: uninsdeletevalue uninsdeletekeyifempty; Check: NoSkinOpener
Root: HKA; Subkey: "Software\Classes\.wsz\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.wsz"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
; How Open with names NeoAmp, and the files it offers it for.
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "NeoAmp"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".wsz"; ValueData: ""

[Run]
Filename: "{app}\neoamp.exe"; Description: "{cm:LaunchProgram,NeoAmp}"; Flags: nowait postinstall skipifsilent

[Code]
// Whether no program opens .wsz files yet, or NeoAmp does already (an earlier install).
function NoSkinOpener: Boolean;
var
  Opener: String;
begin
  Result := not RegQueryStringValue(HKEY_CLASSES_ROOT, '.wsz', '', Opener)
    or (Opener = '') or (Opener = 'NeoAmp.wsz');
end;
