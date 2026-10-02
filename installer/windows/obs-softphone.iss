; SPDX-License-Identifier: GPL-2.0-or-later
; Inno Setup installer for SIP Call-In for OBS (Windows x64).
; Built by scripts\package-windows.ps1:  ISCC /DAppVersion=x.y.z obs-softphone.iss
; Installs into OBS's machine-wide plugin folder:
;   C:\ProgramData\obs-studio\plugins\obs-softphone\bin\64bit\obs-softphone.dll

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Keep this AppId: upgrades and the uninstaller find earlier installs by it.
AppId={{19773B97-493A-4DDC-973D-953E8AFEA44C}
AppName=SIP Call-In for OBS
AppVersion={#AppVersion}
AppPublisher=LastPatriot
AppPublisherURL=https://github.com/LastPatriot/obs-softphone
AppSupportURL=https://github.com/LastPatriot/obs-softphone/issues
DefaultDirName={commonappdata}\obs-studio\plugins\obs-softphone
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=obs-softphone-{#AppVersion}-windows-x64-installer
UninstallDisplayName=SIP Call-In for OBS
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "..\..\dist\obs-softphone\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Messages]
FinishedLabel=SIP Call-In is installed. Start OBS, then open Tools > SIP Call-In... to set it up.
