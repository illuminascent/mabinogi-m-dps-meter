param([switch]$UseDocker,[string]$Image = 'mba-winbuild:latest')
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build-rust.ps1') -UseDocker:$UseDocker -Image $Image -OutputName 'mabi-research.exe'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'capture_epochs.py') -Destination (Join-Path $PSScriptRoot '../dist/capture-epochs.pyw')
Write-Host 'Launch dist/capture-epochs.pyw with Python 3.12+ (Tk included).'
