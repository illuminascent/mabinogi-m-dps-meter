param([switch]$UseDocker,[string]$Image = 'mba-winbuild:latest')
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'rust-task.ps1') -Task Build -Research -UseDocker:$UseDocker -Image $Image -OutputName 'mabi-research.exe'
Write-Host 'Launch dist/mabi-research.exe. This single EXE needs no Python or capture DLL.'
