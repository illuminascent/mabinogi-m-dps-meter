param([switch]$UseDocker,[switch]$LocalFixtures,[string]$Image = 'mba-winbuild:latest')
& (Join-Path $PSScriptRoot 'rust-task.ps1') -Task Test -UseDocker:$UseDocker -LocalFixtures:$LocalFixtures -Image $Image
