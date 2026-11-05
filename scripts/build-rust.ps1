param([switch]$UseDocker,[string]$Image = 'mba-winbuild:latest',[string]$OutputName = 'mabi-meter.exe')
& (Join-Path $PSScriptRoot 'rust-task.ps1') -Task Build -UseDocker:$UseDocker -Image $Image -OutputName $OutputName
