# SPDX-License-Identifier: GPL-2.0-or-later
# Builds the plugin and packages it in dist\ (Windows x64):
#   obs-softphone\bin\64bit\obs-softphone.dll    (OBS's plugin layout)
#   obs-softphone-<ver>-windows-x64.zip           (that folder, zipped)
#   obs-softphone-<ver>-windows-x64-installer.exe (Inno Setup)
#   pwsh scripts\package-windows.ps1
$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
Set-Location $Root

cargo build --release -p obs-softphone
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$Version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
$Dist = Join-Path $Root 'dist'
$Plugin = Join-Path $Dist 'obs-softphone'
Remove-Item -Recurse -Force $Dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$Plugin\bin\64bit", "$Plugin\data" | Out-Null
Copy-Item target\release\obs_softphone.dll "$Plugin\bin\64bit\obs-softphone.dll"
Copy-Item LICENSE "$Plugin\data\LICENSE.txt"

$Name = "obs-softphone-$Version-windows-x64"
Compress-Archive -Path $Plugin -DestinationPath "$Dist\$Name.zip"

$Iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
if (-not (Test-Path $Iscc)) {
    choco install innosetup --no-progress -y | Out-Null
}
& $Iscc /Q "/DAppVersion=$Version" installer\windows\obs-softphone.iss
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }

Get-ChildItem $Dist -File | ForEach-Object { Write-Host $_.FullName }
