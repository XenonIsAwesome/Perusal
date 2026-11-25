# Installs a test-signed Perusal MSIX: trusts its signing certificate (needs an elevated PowerShell), then installs the package.
#   ./install-msix.ps1 -Package Perusal-0.1.0-x64.msix -Cert Perusal-test-signing.cer
# Without this step Windows refuses the package with 0x800B010A ("publisher certificate could not be verified").
param([Parameter(Mandatory)][string]$Package, [Parameter(Mandatory)][string]$Cert)
$ErrorActionPreference = "Stop"
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw "Run this from an elevated (administrator) PowerShell: the certificate goes into LocalMachine\TrustedPeople." }
Import-Certificate -FilePath $Cert -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
Add-AppxPackage -Path $Package
Write-Host "installed; start Perusal from the Start menu"
