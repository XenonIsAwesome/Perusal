# Builds and signs target\Perusal-<version>-x64.msix from the release binaries (needs the Windows SDK: makeappx and signtool).
#   -CertPfx/-CertPassword  sign with your own code-signing certificate (its subject must equal -Publisher)
#   otherwise a self-signed test certificate is created; Perusal.cer is written next to the package and must be trusted
#   (Cert:\LocalMachine\TrustedPeople) on the machine installing it
#   (install-msix.ps1 does that; without it Windows fails with 0x800B010A). The Microsoft Store signs packages itself.
param([string]$Version = "", [string]$BinDir = "target\release", [string]$Publisher = "CN=Perusal", [string]$CertPfx = "", [string]$CertPassword = "")
$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."
if (-not $Version) { $Version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(\d+\.\d+\.\d+)' | Select-Object -First 1).Matches[0].Groups[1].Value }
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64" -Directory -ErrorAction SilentlyContinue | Where-Object { Test-Path "$($_.FullName)\makeappx.exe" } | Sort-Object { [version](Split-Path (Split-Path $_.FullName) -Leaf) } | Select-Object -Last 1
if (-not $sdk) { throw "Windows SDK (makeappx.exe) not found" }
$stage = "$root\target\msix-stage"; Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue; New-Item -ItemType Directory "$stage\Assets" | Out-Null
Copy-Item "$root\$BinDir\perusal.exe", "$root\$BinDir\perusal-cli.exe", "$root\LICENSE-MIT", "$root\LICENSE-APACHE" $stage
& "$PSScriptRoot\make-assets.ps1" -IcoOut "$stage\perusal.ico" -TilesDir "$stage\Assets" | Out-Null; Remove-Item "$stage\perusal.ico"
(Get-Content "$PSScriptRoot\AppxManifest.xml" -Raw).Replace("@VERSION@", "$Version.0").Replace("@PUBLISHER@", $Publisher) | Set-Content "$stage\AppxManifest.xml" -Encoding utf8
$out = "$root\target\Perusal-$Version-x64.msix"; Remove-Item $out -ErrorAction SilentlyContinue
& "$($sdk.FullName)\makeappx.exe" pack /d $stage /p $out /o | Out-Host; if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }
if (-not $CertPfx) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher -KeyUsage DigitalSignature -FriendlyName "Perusal test signing" -CertStoreLocation "Cert:\CurrentUser\My" -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    $CertPassword = [guid]::NewGuid().ToString(); $CertPfx = "$root\target\Perusal-test.pfx"
    Export-PfxCertificate -Cert $cert -FilePath $CertPfx -Password (ConvertTo-SecureString $CertPassword -AsPlainText -Force) | Out-Null
    Export-Certificate -Cert $cert -FilePath "$root\target\Perusal.cer" | Out-Null
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)"
}
$sign = @("sign", "/fd", "SHA256", "/f", $CertPfx); if ($CertPassword) { $sign += @("/p", $CertPassword) }
& "$($sdk.FullName)\signtool.exe" @sign $out | Out-Host; if ($LASTEXITCODE -ne 0) { throw "signtool failed" }
Write-Host "built $out"
