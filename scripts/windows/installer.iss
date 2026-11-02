#ifndef AppVersion
  #error AppVersion must be provided
#endif
#ifndef StageDir
  #error StageDir must be provided
#endif

[Setup]
AppId={{85DBF331-668F-4B84-8AFA-C03CDF8B45B5}
AppName=RV
AppVersion={#AppVersion}
AppPublisher=RV
AppPublisherURL=https://github.com/gitssie/rv
DefaultDirName={localappdata}\Programs\RV
DefaultGroupName=RV
DisableProgramGroupPage=yes
DisableDirPage=no
UsePreviousAppDir=yes
PrivilegesRequired=lowest
#if Architecture == "aarch64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
MinVersion=10.0
OutputDir=..\..\dist
OutputBaseFilename={#ArtifactName}
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\app-icon.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Dirs]
; Application-created data is intentionally outside the uninstall file list.
Name: "{app}\data"; Flags: uninsneveruninstall

[Files]
Source: "{#StageDir}\rv.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\app-icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\RV"; Filename: "{app}\rv.exe"; WorkingDir: "{app}"; IconFilename: "{app}\app-icon.ico"
Name: "{autodesktop}\RV"; Filename: "{app}\rv.exe"; WorkingDir: "{app}"; IconFilename: "{app}\app-icon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\rv.exe"; Description: "{cm:LaunchProgram,RV}"; Flags: nowait postinstall skipifsilent runasoriginaluser
