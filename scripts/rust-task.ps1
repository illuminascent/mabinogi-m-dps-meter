param(
    [ValidateSet('Build','Test','Fetch')][string]$Task = 'Build',
    [switch]$UseDocker,
    [switch]$LocalFixtures,
    [string]$Image = 'mba-winbuild:latest',
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*\.exe$')][string]$OutputName = 'mabi-meter.exe'
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$nativeRoot = Join-Path $repoRoot 'native'
$buildRoot = Join-Path $repoRoot '.build/rust'
$cacheRoot = Join-Path $repoRoot '.build/cargo'
$outputRoot = Join-Path $repoRoot 'dist'
New-Item -ItemType Directory -Force -Path $buildRoot,$cacheRoot,$outputRoot | Out-Null

# No UAC, capture, original helpers, global toolchain install or global config writes.
# Fetch is a separate explicit step. Build and Test use no registry network access.
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$dockerBuild = $UseDocker -or -not $cargoCommand
if (-not (Test-Path -LiteralPath (Join-Path $nativeRoot 'Cargo.lock'))) {
    throw 'Native Cargo.lock is missing. Restore the tracked lockfile before building or fetching dependencies.'
}
$cargoArgs = @(switch ($Task) {
    'Fetch' { @('fetch','--locked') }
    'Build' {
        if ($dockerBuild) { @('build','--locked','--offline','--release','-p','mabi-meter','--target','x86_64-pc-windows-gnu') }
        else { @('build','--locked','--offline','--release','-p','mabi-meter') }
    }
    'Test' {
        @('test','--locked','--offline','--workspace')
        if ($LocalFixtures) { @('--','--include-ignored') }
    }
})

if ($dockerBuild) {
    $networkMode = if ($Task -eq 'Fetch') { 'bridge' } else { 'none' }
    $dockerArgs = @('run','--rm','--network',$networkMode,
        '--mount',"type=bind,source=$nativeRoot,target=/repo/native",
        '--mount',"type=bind,source=$buildRoot,target=/target",
        '--mount',"type=bind,source=$cacheRoot,target=/cargo",
        '--mount',"type=bind,source=$repoRoot/assets/data/combat-names.json,target=/repo/assets/data/combat-names.json,readonly",
        '--workdir','/repo/native','--env','CARGO_TARGET_DIR=/target','--env','CARGO_HOME=/cargo')
    if ($Task -eq 'Build') {
        $dockerArgs += @('--env','CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc',
            '--env','RUSTFLAGS=-C target-feature=+crt-static')
    }
    if ($Task -eq 'Test' -and $LocalFixtures) {
        foreach ($fixture in @('live-native-07.jsonl','ui-name-entry-02.jsonl','rust-current-dummy-20261105-195023.jsonl','rust-identity-profile-20261105-205033.jsonl','jp-appearance-native-fixture.json','jp-identity-research-long-five-self-joins.jsonl','jp-identity-research-long-focused-tcp.jsonl')) {
            $fixturePath = Join-Path $repoRoot "prototypes/static-analysis/$fixture"
            if (-not (Test-Path -LiteralPath $fixturePath)) { throw "Missing private fixture: $fixture" }
            $dockerArgs += @('--mount',"type=bind,source=$fixturePath,target=/repo/prototypes/static-analysis/$fixture,readonly")
        }
    }
    $dockerArgs += @($Image,'cargo')
    & docker @dockerArgs @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Rust $Task failed. No capture was started." }
} else {
    $previousTarget = $env:CARGO_TARGET_DIR
    $previousCache = $env:CARGO_HOME
    $previousFlags = $env:RUSTFLAGS
    try {
        $env:CARGO_TARGET_DIR = $buildRoot
        $env:CARGO_HOME = $cacheRoot
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        Push-Location $nativeRoot
        try {
            & $cargoCommand.Source @cargoArgs
            if ($LASTEXITCODE -ne 0) { throw "Rust $Task failed." }
        } finally { Pop-Location }
    } finally {
        $env:CARGO_TARGET_DIR = $previousTarget
        $env:CARGO_HOME = $previousCache
        $env:RUSTFLAGS = $previousFlags
    }
}
if ($Task -eq 'Build') {
    $relativeExe = if ($dockerBuild) { 'x86_64-pc-windows-gnu/release/mabi-meter.exe' } else { 'release/mabi-meter.exe' }
    $artifact = Join-Path $buildRoot $relativeExe
    if (-not (Test-Path -LiteralPath $artifact)) { throw 'Build returned success without the expected Windows EXE.' }
    Copy-Item -LiteralPath $artifact -Destination (Join-Path $outputRoot $OutputName)
    Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $outputRoot $OutputName)
}
