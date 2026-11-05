param([switch]$UseDocker,[string]$Image = 'mba-winbuild:latest')
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$nativeRoot = Join-Path $repoRoot 'native/capture'
$targetRoot = Join-Path $repoRoot '.build/capture'
$outputRoot = Join-Path $repoRoot 'dist'
New-Item -ItemType Directory -Force -Path $targetRoot,$outputRoot | Out-Null
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$dockerBuild = $UseDocker -or -not $cargoCommand
if ($dockerBuild) {
    # Mount only our capture source and output, never reference samples.
    & docker run --rm --network none --mount "type=bind,source=$nativeRoot,target=/source" --mount "type=bind,source=$targetRoot,target=/target" --workdir /source --env CARGO_TARGET_DIR=/target --env CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc --env 'RUSTFLAGS=-C target-feature=+crt-static' $Image cargo build --offline --locked --release --target x86_64-pc-windows-gnu
    if ($LASTEXITCODE -ne 0) { throw 'Windows capture DLL build failed' }
    $artifact = Join-Path $targetRoot 'x86_64-pc-windows-gnu/release/mabi_capture.dll'
} else {
    $previousTarget = $env:CARGO_TARGET_DIR
    $previousFlags = $env:RUSTFLAGS
    try {
        $env:CARGO_TARGET_DIR = $targetRoot
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        & $cargoCommand.Source build --offline --locked --release --manifest-path (Join-Path $nativeRoot 'Cargo.toml')
        if ($LASTEXITCODE -ne 0) { throw 'Windows capture DLL build failed' }
    } finally {
        $env:CARGO_TARGET_DIR = $previousTarget
        $env:RUSTFLAGS = $previousFlags
    }
    $artifact = Join-Path $targetRoot 'release/mabi_capture.dll'
}
if (-not (Test-Path -LiteralPath $artifact)) { throw 'Expected Windows capture DLL was not produced.' }
Copy-Item -LiteralPath $artifact -Destination (Join-Path $outputRoot 'mabi_capture.dll')
Write-Output (Join-Path $outputRoot 'mabi_capture.dll')
