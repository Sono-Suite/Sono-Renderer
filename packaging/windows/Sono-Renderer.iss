#ifndef AppVersion
  #error AppVersion must be provided by package-windows.ps1
#endif
#ifndef SourceDir
  #error SourceDir must be provided by package-windows.ps1
#endif
#ifndef OutputDir
  #error OutputDir must be provided by package-windows.ps1
#endif

#define AppName "Sono-Renderer"
#define AppId "{4B8AC640-9D6D-4A8A-9DC1-EBC0D87052F4}"

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Nexint
DefaultDirName={localappdata}\Programs\Sono-Renderer
DefaultGroupName=Sono-Renderer
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
UsePreviousAppDir=yes
CloseApplications=yes
RestartApplications=no
OutputDir={#OutputDir}
OutputBaseFilename=Sono-Renderer-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Sono-Renderer {#AppVersion}
UninstallDisplayIcon={app}\Sono-Renderer.exe
VersionInfoVersion={#AppVersion}
VersionInfoCompany=Nexint
VersionInfoDescription=Sono-Renderer installer
VersionInfoProductName=Sono-Renderer
VersionInfoProductVersion={#AppVersion}
VersionInfoCopyright=Copyright (C) Nexint

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Sono-Renderer"; Filename: "{app}\Sono-Renderer.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Sono-Renderer"; Filename: "{app}\Sono-Renderer.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\Sono-Renderer.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\Sono-Renderer.exe"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\Sono-Renderer.exe"; ValueType: string; ValueName: "Path"; ValueData: "{app}"; Flags: uninsdeletekey

[Run]
Filename: "{app}\Sono-Renderer.exe"; Description: "Launch Sono-Renderer"; Flags: postinstall nowait skipifsilent
