# SPDX-License-Identifier: GPL-2.0-or-later
# Fetches and builds the native dependencies into third_party\ (Windows x64).
# Run from a Visual Studio x64 developer shell (cl, msbuild, dumpbin, lib):
#   - Opus, static, /MD (CMake)
#   - pjproject, static, /MD (MSBuild, "libpjproject" aggregate): TLS through
#     Windows Schannel, Opus, no video
#   - Qt 6 headers and import libraries from obs-deps (matching OBS)
#   - import libraries for obs.dll and obs-frontend-api.dll, generated from
#     the official OBS release
#   pwsh scripts\bootstrap-windows.ps1
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Root = Split-Path $PSScriptRoot -Parent
$TP = Join-Path $Root 'third_party'
New-Item -ItemType Directory -Force $TP | Out-Null

$PjVersion = '2.17'
$OpusVersion = '1.5.2'
$OpusSha256 = '65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1' # xiph SHA256SUMS.txt
$ObsVersion = '32.2.2'
$ObsSha256 = '4d6e40e3ab155f56b30de517380566a206d74b63cdf5ad49aa596924768f97e1'   # GitHub release asset digest
$Qt6DepsVersion = '2026-07-15'                                                       # obs-studio 32.2.2 CMakePresets.json
$Qt6DepsSha256 = '7c7f985711d80467bdc1795b6592275a27d5b0e5a2c7a61db1f2c1d08d6a5579'
# Bump when the build options below change, to force a rebuild.
$BuildRev = 'v1-schannel'

function Get-Verified([string]$Url, [string]$OutFile, [string]$Sha256) {
    Invoke-WebRequest -Uri $Url -OutFile $OutFile
    $actual = (Get-FileHash -Algorithm SHA256 $OutFile).Hash
    if ($actual -ne $Sha256.ToUpperInvariant()) {
        throw "Checksum mismatch for $Url`n  expected $Sha256`n  got      $actual"
    }
}

# Extracts only the zip entries whose path starts with one of $Prefixes.
function Expand-Selected([string]$Zip, [string]$Dest, [string[]]$Prefixes) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        foreach ($entry in $archive.Entries) {
            $name = $entry.FullName -replace '\\', '/'
            if (-not ($Prefixes | Where-Object { $name.StartsWith($_) })) { continue }
            if ($name.EndsWith('/')) { continue }
            $target = Join-Path $Dest $name
            New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
        }
    } finally {
        $archive.Dispose()
    }
}

function Invoke-Checked([string]$What, [scriptblock]$Block) {
    & $Block
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

# --- Opus ------------------------------------------------------------------
$OpusPrefix = Join-Path $TP 'opus-x64'
if (-not (Test-Path "$OpusPrefix\.rev-$BuildRev")) {
    $tarball = Join-Path $TP "opus-$OpusVersion.tar.gz"
    Get-Verified "https://downloads.xiph.org/releases/opus/opus-$OpusVersion.tar.gz" $tarball $OpusSha256
    $src = Join-Path $TP 'opus-src'
    Remove-Item -Recurse -Force $src, $OpusPrefix -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $src | Out-Null
    Invoke-Checked 'extract opus' { tar -xzf $tarball -C $src --strip-components 1 }
    Remove-Item $tarball
    $build = Join-Path $TP 'build\opus-x64'
    Invoke-Checked 'configure opus' {
        cmake -S $src -B $build -A x64 -DCMAKE_INSTALL_PREFIX="$OpusPrefix" `
            -DBUILD_SHARED_LIBS=OFF -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF `
            -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL
    }
    Invoke-Checked 'build opus' { cmake --build $build --config Release --target install }
    Remove-Item -Recurse -Force $build
    New-Item -ItemType File "$OpusPrefix\.rev-$BuildRev" | Out-Null
    Write-Host 'opus: built'
} else {
    Write-Host 'opus: already built'
}

# --- pjproject -------------------------------------------------------------
# Built in its source tree; the tree stays (headers + libs) for the Rust build.
$Pj = Join-Path $TP 'pjproject-x64'
if (-not (Test-Path "$Pj\.rev-$BuildRev")) {
    Remove-Item -Recurse -Force $Pj -ErrorAction SilentlyContinue
    Invoke-Checked 'clone pjproject' {
        git clone -q --depth 1 --branch $PjVersion https://github.com/pjsip/pjproject $Pj
    }
    @'
/* obs-softphone: one account, one call, no video. */
#define PJ_HAS_SSL_SOCK            1
/* TLS through Windows Schannel: system trust store, no OpenSSL. */
#define PJ_SSL_SOCK_IMP            PJ_SSL_SOCK_IMP_SCHANNEL
#define PJMEDIA_HAS_OPUS_CODEC     1
#define PJSUA_MAX_CALLS            4
#define PJSUA_MAX_ACC              2
#define PJMEDIA_HAS_VIDEO          0
#define PJMEDIA_CONF_USE_SWITCH_BOARD 0
/* Answer with our codec order (opus, G722, PCMU, PCMA), not the offerer's. */
#define PJMEDIA_SDP_NEG_PREFER_REMOTE_CODEC_ORDER 0
'@ | Set-Content -Encoding ascii "$Pj\pjlib\include\pj\config_site.h"

    # Opus headers for pjmedia-codec: CL adds options to every cl.exe run.
    $env:CL = "/I`"$OpusPrefix\include`""
    Invoke-Checked 'build pjproject' {
        msbuild "$Pj\pjproject-vs14.sln" /t:libpjproject /m /v:minimal /nologo `
            /p:Configuration=Release-Dynamic /p:Platform=x64 `
            /p:PlatformToolset=v143 /p:WindowsTargetPlatformVersion=10.0
    }
    Remove-Item Env:\CL
    $lib = Get-ChildItem "$Pj\pjsip-apps\lib\libpjproject-*Release-Dynamic.lib" | Select-Object -First 1
    if (-not $lib) { throw 'pjproject: libpjproject .lib not found' }
    New-Item -ItemType File "$Pj\.rev-$BuildRev" | Out-Null
    Write-Host "pjproject: built $($lib.Name)"
} else {
    Write-Host 'pjproject: already built'
}

# --- Qt 6 (obs-deps): headers and import libraries ---------------------------
$Qt = Join-Path $TP 'obs-deps-qt6'
if (-not (Test-Path "$Qt\lib\Qt6Widgets.lib")) {
    $zip = Join-Path $TP 'qt6.zip'
    Get-Verified "https://github.com/obsproject/obs-deps/releases/download/$Qt6DepsVersion/windows-deps-qt6-$Qt6DepsVersion-x64.zip" $zip $Qt6DepsSha256
    Remove-Item -Recurse -Force $Qt -ErrorAction SilentlyContinue
    Expand-Selected $zip $Qt @('include/QtCore/', 'include/QtGui/', 'include/QtWidgets/',
        'lib/Qt6Core.lib', 'lib/Qt6Gui.lib', 'lib/Qt6Widgets.lib', 'mkspecs/win32-msvc/')
    Remove-Item $zip
    Write-Host 'qt6: headers and libs ready'
} else {
    Write-Host 'qt6: already present'
}

# --- OBS import libraries ----------------------------------------------------
# The OBS release ships DLLs only; generate .lib files from their exports.
$Obs = Join-Path $TP 'obs-x64'
if (-not (Test-Path "$Obs\lib\obs-frontend-api.lib")) {
    $zip = Join-Path $TP 'obs.zip'
    Get-Verified "https://github.com/obsproject/obs-studio/releases/download/$ObsVersion/OBS-Studio-$ObsVersion-Windows-x64.zip" $zip $ObsSha256
    $tmp = Join-Path $TP 'obs-tmp'
    Remove-Item -Recurse -Force $tmp, $Obs -ErrorAction SilentlyContinue
    Expand-Selected $zip $tmp @('bin/64bit/obs.dll', 'bin/64bit/obs-frontend-api.dll')
    Remove-Item $zip
    New-Item -ItemType Directory -Force "$Obs\lib" | Out-Null
    foreach ($name in 'obs', 'obs-frontend-api') {
        $dll = Join-Path $tmp "bin\64bit\$name.dll"
        if (-not (Test-Path $dll)) { throw "$name.dll not found in the OBS release" }
        # dumpbin lists exports as "ordinal hint RVA name"; keep the names.
        $exports = dumpbin /nologo /exports $dll | ForEach-Object {
            if ($_ -match '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]{8}\s+(\S+)') { $Matches[1] }
        }
        if ($exports.Count -lt 10) { throw "too few exports read from $name.dll" }
        $def = Join-Path $Obs "lib\$name.def"
        @("LIBRARY $name", 'EXPORTS') + ($exports | ForEach-Object { "    $_" }) | Set-Content -Encoding ascii $def
        Invoke-Checked "lib $name" { lib /nologo /def:$def /machine:x64 /out:"$Obs\lib\$name.lib" }
        Write-Host "obs: $name.lib ($($exports.Count) exports)"
    }
    Remove-Item -Recurse -Force $tmp
} else {
    Write-Host 'obs: import libs already present'
}

Write-Host 'bootstrap done (windows x64)'
