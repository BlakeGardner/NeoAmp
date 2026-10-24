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
; Tells Explorer about the file associations below.
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
Name: "contextmenu"; Description: "Add ""Play in NeoAmp"" and ""Enqueue in NeoAmp"" to folders' right-click menu, and ""Enqueue in NeoAmp"" to audio files' and playlists'"; GroupDescription: "Explorer:"

[Files]
Source: "{#Binary}"; DestDir: "{app}"; DestName: "neoamp.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\NeoAmp"; Filename: "{app}\neoamp.exe"; Comment: "Play music and internet radio with classic Winamp skins"
Name: "{autodesktop}\NeoAmp"; Filename: "{app}\neoamp.exe"; Tasks: desktopicon

[Registry]
; Audio files, playlists and classic skins (.wsz) open in NeoAmp: listed under Open with and in
; Settings > Apps > Default apps, where NeoAmp can be made their default, and opened on a
; double-click when no other program opens them already (Windows' own player, or Winamp, say,
; which keep theirs). A file opened while NeoAmp runs goes to the running NeoAmp. Explorer starts
; NeoAmp once for each of the files opened together, and the running copy gathers them into one
; playlist (src/instance.rs); MultiSelectModel=Player lets it open up to 100 at once instead of
; 15. MP4 files are left out, being mostly films. HKA is the current user's keys, or the
; machine's when installing for all users.
; The file types, as Explorer names them.
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp3"; ValueType: string; ValueName: ""; ValueData: "MP3 audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp3\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp3\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp3\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp2"; ValueType: string; ValueName: ""; ValueData: "MP1 and MP2 audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp2\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp2\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.mp2\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.aac"; ValueType: string; ValueName: ""; ValueData: "AAC audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.aac\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.aac\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.aac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.m4a"; ValueType: string; ValueName: ""; ValueData: "MPEG-4 audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.m4a\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.m4a\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.m4a\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.wav"; ValueType: string; ValueName: ""; ValueData: "WAV audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.wav\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.wav\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.wav\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.aiff"; ValueType: string; ValueName: ""; ValueData: "AIFF audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.aiff\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.aiff\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.aiff\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.flac"; ValueType: string; ValueName: ""; ValueData: "FLAC audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.flac\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.flac\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.flac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.ogg"; ValueType: string; ValueName: ""; ValueData: "Ogg Vorbis audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.ogg\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.ogg\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.ogg\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.m3u"; ValueType: string; ValueName: ""; ValueData: "M3U playlist"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.m3u\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.m3u\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.m3u\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.pls"; ValueType: string; ValueName: ""; ValueData: "PLS playlist"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.pls\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.pls\shell\open"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\NeoAmp.pls\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz"; ValueType: string; ValueName: ""; ValueData: "Winamp classic skin"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\Classes\NeoAmp.wsz\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""
; Each extension: its type, unless another program opens it already, and NeoAmp under Open
; with. Uninstalling removes the type only where it is still NeoAmp's (CurUninstallStepChanged
; below), so a program that has taken the extension over since keeps it. It then goes through
; these in reverse, so an extension's key, if NeoAmp leaves it empty, is removed after its
; OpenWithProgids.
Root: HKA; Subkey: "Software\Classes\.mp3"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.mp3"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.mp3', 'NeoAmp.mp3')
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.mp3"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp2"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.mp2"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.mp2', 'NeoAmp.mp2')
Root: HKA; Subkey: "Software\Classes\.mp2\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.mp2"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp1"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.mp2"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.mp1', 'NeoAmp.mp2')
Root: HKA; Subkey: "Software\Classes\.mp1\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.mp2"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aac"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.aac"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.aac', 'NeoAmp.aac')
Root: HKA; Subkey: "Software\Classes\.aac\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.aac"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m4a"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.m4a"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.m4a', 'NeoAmp.m4a')
Root: HKA; Subkey: "Software\Classes\.m4a\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.m4a"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m4b"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.m4a"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.m4b', 'NeoAmp.m4a')
Root: HKA; Subkey: "Software\Classes\.m4b\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.m4a"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wav"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.wav"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.wav', 'NeoAmp.wav')
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.wav"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wave"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.wav"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.wave', 'NeoAmp.wav')
Root: HKA; Subkey: "Software\Classes\.wave\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.wav"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aiff"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.aiff"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.aiff', 'NeoAmp.aiff')
Root: HKA; Subkey: "Software\Classes\.aiff\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.aiff"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aif"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.aiff"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.aif', 'NeoAmp.aiff')
Root: HKA; Subkey: "Software\Classes\.aif\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.aiff"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aifc"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.aiff"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.aifc', 'NeoAmp.aiff')
Root: HKA; Subkey: "Software\Classes\.aifc\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.aiff"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.flac"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.flac"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.flac', 'NeoAmp.flac')
Root: HKA; Subkey: "Software\Classes\.flac\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.flac"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ogg"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.ogg"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.ogg', 'NeoAmp.ogg')
Root: HKA; Subkey: "Software\Classes\.ogg\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.ogg"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.oga"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.ogg"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.oga', 'NeoAmp.ogg')
Root: HKA; Subkey: "Software\Classes\.oga\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.ogg"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m3u"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.m3u"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.m3u', 'NeoAmp.m3u')
Root: HKA; Subkey: "Software\Classes\.m3u\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.m3u"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m3u8"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.m3u"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.m3u8', 'NeoAmp.m3u')
Root: HKA; Subkey: "Software\Classes\.m3u8\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.m3u"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.pls"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.pls"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.pls', 'NeoAmp.pls')
Root: HKA; Subkey: "Software\Classes\.pls\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.pls"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wsz"; ValueType: string; ValueName: ""; ValueData: "NeoAmp.wsz"; Flags: uninsdeletekeyifempty; Check: NoOtherOpener('.wsz', 'NeoAmp.wsz')
Root: HKA; Subkey: "Software\Classes\.wsz\OpenWithProgids"; ValueType: string; ValueName: "NeoAmp.wsz"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
; How Open with names NeoAmp, and the files it offers it for.
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "NeoAmp"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".mp2"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".mp1"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".aac"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".m4a"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".m4b"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".wav"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".wave"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".aiff"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".aif"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".aifc"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".flac"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".ogg"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".oga"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".m3u"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".m3u8"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".pls"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\neoamp.exe\SupportedTypes"; ValueType: string; ValueName: ".wsz"; ValueData: ""
; NeoAmp in Settings > Apps > Default apps, where it can be made the default for each type.
Root: HKA; Subkey: "Software\NeoAmp"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\NeoAmp\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "NeoAmp"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\NeoAmp\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Play music and internet radio with classic Winamp skins"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: "{app}\neoamp.exe,0"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mp3"; ValueData: "NeoAmp.mp3"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mp2"; ValueData: "NeoAmp.mp2"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mp1"; ValueData: "NeoAmp.mp2"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".aac"; ValueData: "NeoAmp.aac"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m4a"; ValueData: "NeoAmp.m4a"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m4b"; ValueData: "NeoAmp.m4a"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wav"; ValueData: "NeoAmp.wav"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wave"; ValueData: "NeoAmp.wav"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".aiff"; ValueData: "NeoAmp.aiff"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".aif"; ValueData: "NeoAmp.aiff"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".aifc"; ValueData: "NeoAmp.aiff"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".flac"; ValueData: "NeoAmp.flac"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ogg"; ValueData: "NeoAmp.ogg"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".oga"; ValueData: "NeoAmp.ogg"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m3u"; ValueData: "NeoAmp.m3u"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m3u8"; ValueData: "NeoAmp.m3u"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".pls"; ValueData: "NeoAmp.pls"
Root: HKA; Subkey: "Software\NeoAmp\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wsz"; ValueData: "NeoAmp.wsz"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "NeoAmp"; ValueData: "Software\NeoAmp\Capabilities"; Flags: uninsdeletevalue
; The right-click menus (the contextmenu task): Play and Enqueue in NeoAmp for folders, and
; Enqueue in NeoAmp for audio files and playlists, whichever program opens them.
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.play"; ValueType: string; ValueName: ""; ValueData: "&Play in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.play"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.play"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.play\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp3\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp3\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp3\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp3\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp2\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp2\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp2\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp2\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp1\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp1\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp1\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp1\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aac\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4a\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4a\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4a\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4a\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4b\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4b\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4b\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4b\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wav\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wav\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wav\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wav\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wave\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wave\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wave\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wave\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aiff\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aiff\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aiff\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aiff\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aif\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aif\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aif\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aif\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aifc\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aifc\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aifc\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.aifc\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flac\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flac\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ogg\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ogg\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ogg\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ogg\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.oga\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.oga\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.oga\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.oga\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u8\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u8\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u8\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m3u8\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.pls\shell\NeoAmp.enqueue"; ValueType: string; ValueName: ""; ValueData: "&Enqueue in NeoAmp"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.pls\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\neoamp.exe,0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.pls\shell\NeoAmp.enqueue"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.pls\shell\NeoAmp.enqueue\command"; ValueType: string; ValueName: ""; ValueData: """{app}\neoamp.exe"" --enqueue ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\neoamp.exe"; Description: "{cm:LaunchProgram,NeoAmp}"; Flags: nowait postinstall skipifsilent

[Code]
// Whether no program opens files with the extension Ext yet, or NeoAmp does already, as
// ProgId (an earlier install).
function NoOtherOpener(Ext, ProgId: String): Boolean;
var
  Opener: String;
begin
  Result := not RegQueryStringValue(HKEY_CLASSES_ROOT, Ext, '', Opener)
    or (Opener = '') or (Opener = ProgId);
end;

// Before uninstalling removes NeoAmp's keys: take NeoAmp's type off each extension that still
// has it. One another program has taken over since is left alone, so that it keeps opening
// those files. The extensions and their types come from NeoAmp's Default apps entry, which is
// removed only afterwards.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Root, I: Integer;
  Exts: TArrayOfString;
  ProgId, Opener: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;
  // The keys HKA meant when installing.
  if IsAdminInstallMode then
    Root := HKEY_LOCAL_MACHINE
  else
    Root := HKEY_CURRENT_USER;
  if not RegGetValueNames(Root, 'Software\NeoAmp\Capabilities\FileAssociations', Exts) then
    Exit;
  for I := 0 to GetArrayLength(Exts) - 1 do
    if RegQueryStringValue(Root, 'Software\NeoAmp\Capabilities\FileAssociations', Exts[I], ProgId)
      and RegQueryStringValue(Root, 'Software\Classes\' + Exts[I], '', Opener)
      and (Opener = ProgId) then
      RegDeleteValue(Root, 'Software\Classes\' + Exts[I], '');
end;
