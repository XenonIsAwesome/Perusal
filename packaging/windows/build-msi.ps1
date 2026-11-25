# Builds target\Perusal-<version>-x64.msi from the release binaries. Needs the WiX v5 CLI: dotnet tool install --global wix --version 5.0.2
param([string]$Version = "", [string]$BinDir = "target\release")
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."
if (-not $Version) { $Version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(\d+\.\d+\.\d+)' | Select-Object -First 1).Matches[0].Groups[1].Value }
$out = "$root\target\Perusal-$Version-x64.msi"
$env:Path += ";$env:USERPROFILE\.dotnet\tools"
wix build -arch x64 -d Version=$Version -d BinDir="$root\$BinDir" -d IconFile="$root\crates\ui\assets\perusal.ico" -d LicenseDir="$root" -o $out "$PSScriptRoot\perusal.wxs"
if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
Write-Host "built $out"
