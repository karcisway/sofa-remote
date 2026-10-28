# Builds everything into release/:
#   sofa-remote-windows-x64.exe   (cargo)
#   sofa-remote-linux-x64         (cargo in Docker; skipped when Docker isn't running)
#   sofa-remote-android.apk       (gradle; skipped when the Android SDK isn't set up)
#
#   .\release.ps1                 build all
#   .\release.ps1 -Skip linux,android
param([string[]]$Skip = @())

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$out = Join-Path $root 'release'
New-Item -ItemType Directory -Force $out | Out-Null
Set-Location $root

function Step($name) { Write-Host "`n== $name" -ForegroundColor Cyan }

if ('windows' -notin $Skip) {
    Step 'Windows'
    # Bundles the gamepad driver installer into the exe; without it the exe downloads it on first use
    try { & (Join-Path $root 'vendor/fetch.ps1') } catch { Write-Warning "Gamepad driver not bundled: $_" }
    cargo build --release
    if ($LASTEXITCODE) { throw 'cargo build failed' }
    Copy-Item target/release/sofa-remote.exe (Join-Path $out 'sofa-remote-windows-x64.exe') -Force
}

if ('linux' -notin $Skip) {
    Step 'Linux (Docker)'
    docker info *> $null
    if ($LASTEXITCODE) {
        Write-Warning 'Docker is not running: skipping the Linux build'
    } else {
        docker run --rm -v "${root}:/work" -w /work -e CARGO_TARGET_DIR=/work/target-linux `
            rust:1.93-bullseye cargo build --release
        if ($LASTEXITCODE) { throw 'Linux build failed' }
        Copy-Item target-linux/release/sofa-remote (Join-Path $out 'sofa-remote-linux-x64') -Force
    }
}

if ('android' -notin $Skip) {
    Step 'Android'
    Push-Location android
    try {
        .\gradlew.bat assembleRelease   # copies the APK to release/ by itself
        if ($LASTEXITCODE) { throw 'gradle build failed' }
    } finally { Pop-Location }
}

Step 'Done'
Get-ChildItem $out -File | Format-Table Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } } -AutoSize
