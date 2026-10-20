; Inno Setup script for the Glowing Cat Software Installer.
; Produces a self-contained Setup .exe.
;
; The build passes these via /D on the ISCC command line:
;   /DAppVersion=0.1.0        - version string
;   /DSourceExe=path\to.exe   - the built glowing-cat-installer.exe
;   /DIconsDir=resources\icons- directory of project icons to bundle
;   /DArchLabel=x64|arm64     - used in the output filename
;   /DArch=x64|arm64          - target architecture
;
; Example:
;   iscc /DAppVersion=0.1.0 ^
;        /DSourceExe=target\x86_64-pc-windows-msvc\release\glowing-cat-installer.exe ^
;        /DIconsDir=resources\icons /DArchLabel=x64 /DArch=x64 packaging\installer.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\glowing-cat-installer.exe"
#endif
#ifndef IconsDir
  #define IconsDir "..\resources\icons"
#endif
#ifndef ArchLabel
  #define ArchLabel "x64"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\out"
#endif

; Inno resolves SetupIconFile (and other relative paths) against the directory
; containing THIS script (packaging\), not the ISCC working directory. The CI
; passes repo-root-relative values for SourceExe / IconsDir, so anchor them at
; the repo root (one level up from this script) to make resolution independent
; of the invocation directory. Absolute values passed on the command line are
; left unchanged.
#define RepoRoot AddBackslash(ExtractFilePath(SourcePath)) + "..\"
#define ResolvedSourceExe (Pos(":", SourceExe) > 0 ? SourceExe : RepoRoot + SourceExe)
#define ResolvedIconsDir (Pos(":", IconsDir) > 0 ? IconsDir : RepoRoot + IconsDir)
#define ResolvedOutputDir (Pos(":", OutputDir) > 0 ? OutputDir : RepoRoot + OutputDir)

[Setup]
AppId={{A1B2C3D4-5E6F-4A7B-8C9D-0E1F2A3B4C5D}
AppName=Glowing Cat Software Installer
AppVersion={#AppVersion}
AppPublisher=Richard Lesh
DefaultDirName={autopf}\Glowing Cat Software Installer
DefaultGroupName=Glowing Cat Software Installer
DisableProgramGroupPage=yes
UninstallDisplayName=Glowing Cat Software Installer
; OutputDir can be overridden with /DOutputDir=<abs path>; otherwise it is
; anchored at the repo root (matching the `out\` the CI signs/uploads from)
; rather than resolved relative to this script's directory.
OutputDir={#ResolvedOutputDir}
OutputBaseFilename=GlowingCatInstaller-{#AppVersion}-{#ArchLabel}-Setup
SetupIconFile={#ResolvedIconsDir}\app_icon.ico
UninstallDisplayIcon={app}\glowing-cat-installer.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; arm64 installs into the 64-bit location and runs native on ARM devices;
; x64 uses the standard 64-bit mode.
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "{#ResolvedSourceExe}"; DestDir: "{app}"; DestName: "glowing-cat-installer.exe"; Flags: ignoreversion
; Bundle the project icons next to the exe (loaded from resources\icons at runtime).
Source: "{#ResolvedIconsDir}\*"; DestDir: "{app}\resources\icons"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

[Icons]
Name: "{group}\Glowing Cat Software Installer"; Filename: "{app}\glowing-cat-installer.exe"
Name: "{autodesktop}\Glowing Cat Software Installer"; Filename: "{app}\glowing-cat-installer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\glowing-cat-installer.exe"; Description: "Launch Glowing Cat Software Installer"; Flags: nowait postinstall skipifsilent
