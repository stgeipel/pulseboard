#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z to ISCC
#endif

[Setup]
AppId={{9BBB0788-0931-499B-843B-BEB4CEE3B428}
AppName=Pulseboard
AppVersion={#AppVersion}
AppMutex=Local\Pulseboard.Desktop
DefaultDirName={localappdata}\Programs\Pulseboard
DefaultGroupName=Pulseboard
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
SourceDir=..
OutputDir=dist
OutputBaseFilename=Pulseboard-{#AppVersion}-Windows-x64-Setup
SetupIconFile=installer\pulseboard.ico
UninstallDisplayIcon={app}\pulseboard.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "target\release\pulseboard.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Pulseboard"; Filename: "{app}\pulseboard.exe"; AppUserModelID: "Pulseboard.Desktop"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Pulseboard"; ValueType: none; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\pulseboard.exe"; Description: "{cm:LaunchProgram,Pulseboard}"; Flags: nowait postinstall skipifsilent
