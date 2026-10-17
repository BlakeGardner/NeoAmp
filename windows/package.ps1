# Builds NeoAmp for 64-bit Windows and packages it two ways, in target\windows\:
#
#   neoamp-v<version>-windows-x86_64-setup.exe   the installer, from windows\neoamp.iss
#   neoamp-v<version>-windows-x86_64.zip         the program and README.md, to run without
#                                                installing
#
# It runs on Windows, from anywhere in this repository, in Windows PowerShell or PowerShell 7:
#
#   powershell -ExecutionPolicy Bypass -File windows\package.ps1
#
# It needs Rust's MSVC toolchain and Inno Setup 6 (https://jrsoftware.org/isdl.php, or
# `winget install JRSoftware.InnoSetup`). It finds Inno Setup's compiler, ISCC.exe, where
# Inno Setup installs it, or at the path in $env:ISCC.
#
# The installer and the program are not signed, so Windows' SmartScreen warns before the
# installer runs the first time (see "Windows" in docs/GUIDE.md). The Release workflow runs this
# for each GitHub release (.github/workflows/windows.yml).

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$target = 'x86_64-pc-windows-msvc'
$version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages[0].version
if ($LASTEXITCODE -ne 0) { throw 'package.ps1: cargo metadata failed' }
$name = "neoamp-v$version-windows-x86_64"
$out = Join-Path $root 'target\windows'

$iscc = $env:ISCC
if (-not $iscc) {
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) {
    throw 'package.ps1: Inno Setup 6 makes the installer: install it from https://jrsoftware.org/isdl.php, or set $env:ISCC to its ISCC.exe'
}

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory $out | Out-Null

# The C runtime is linked in, so NeoAmp runs without the Visual C++ Redistributable installed.
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target $target
if ($LASTEXITCODE -ne 0) { throw 'package.ps1: cargo build failed' }
$exe = Join-Path $root "target\$target\release\neoamp.exe"

& $iscc /Q "/DAppVersion=$version" "/DBinary=$exe" "/DOutputDir=$out" (Join-Path $PSScriptRoot 'neoamp.iss')
if ($LASTEXITCODE -ne 0) { throw 'package.ps1: Inno Setup could not make the installer' }

$folder = Join-Path $out $name
New-Item -ItemType Directory $folder | Out-Null
Copy-Item $exe, (Join-Path $root 'README.md') $folder
Compress-Archive -Path $folder -DestinationPath (Join-Path $out "$name.zip")
Remove-Item -Recurse -Force $folder

Write-Output "Built $out\$name-setup.exe and $out\$name.zip"
