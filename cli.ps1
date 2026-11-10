# Sofa Remote developer commands (Windows). Use through cli.bat: "cli <command> [args]".
param([string]$Command = 'help', [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest = @())

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
Set-Location $root
$exe = Join-Path $root 'target\release\sofa-remote.exe'

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }
function Run($what) {
    # run a native command given as a script block; stop when it fails
    & $what
    if ($LASTEXITCODE) { Fail "failed (exit code $LASTEXITCODE)" }
}

function Version { (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value }

function Build {
    Step 'Windows build (target\release\sofa-remote.exe)'
    # bundles the gamepad driver installer into the exe when it can be fetched
    try { & (Join-Path $root 'vendor\fetch.ps1') } catch { Write-Warning "Gamepad driver not bundled: $_" }
    Run { cargo build --release }
}

function Apk {
    Step 'Android APK (release\sofa-remote-android.apk)'
    Push-Location android
    try { Run { .\gradlew.bat assembleRelease } } finally { Pop-Location }
}

function Stop-Running {
    Get-Process sofa-remote -ErrorAction SilentlyContinue | Stop-Process -Force
}

function Release([string]$version) {
    if ($version -notmatch '^\d+\.\d+\.\d+$') { Fail "usage: cli release X.Y.Z   (current version: $(Version))" }
    $tag = "v$version"
    $branch = (git rev-parse --abbrev-ref HEAD).Trim()
    if ($branch -ne 'main') { Fail "releases are made from main (you are on $branch)" }
    if (git status --porcelain) { Fail 'commit or stash your changes first (git status shows uncommitted work)' }
    if (git tag --list $tag) { Fail "tag $tag already exists: pick a new version" }
    Run { git fetch --tags origin }
    if (git ls-remote --tags origin "refs/tags/$tag") { Fail "tag $tag already exists on GitHub: pick a new version" }

    Step "Version $(Version) -> $version"
    $cargo = Get-Content Cargo.toml -Raw
    $cargo = ([regex]'(?m)^version = ".+"').Replace($cargo, "version = `"$version`"", 1)
    Set-Content Cargo.toml $cargo -NoNewline
    $gradlePath = 'android\app\build.gradle.kts'
    $gradle = Get-Content $gradlePath -Raw
    $code = [int]([regex]'versionCode = (\d+)').Match($gradle).Groups[1].Value + 1
    $gradle = ([regex]'versionCode = \d+').Replace($gradle, "versionCode = $code", 1)
    $gradle = ([regex]'versionName = ".+"').Replace($gradle, "versionName = `"$version`"", 1)
    Set-Content $gradlePath $gradle -NoNewline

    Step 'Tests'
    Run { cargo test --release }   # also updates Cargo.lock
    Apk

    Step 'Ready to publish'
    git --no-pager diff --stat
    Write-Host "`nThis commits the version bump, pushes main and pushes tag $tag;"
    Write-Host 'GitHub Actions then builds all files and publishes the release.'
    $answer = Read-Host 'Type yes to publish'
    if ($answer -ne 'yes') {
        Write-Host 'Stopped. The version bump is left uncommitted (git checkout -- . undoes it).'
        exit 1
    }
    Run { git add Cargo.toml Cargo.lock $gradlePath }
    Run { git commit -m "Bump version to $version" }
    Run { git push origin main }
    Run { git tag $tag }
    Run { git push origin $tag }

    $repo = (git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/'
    Step 'Pushed'
    Write-Host "Build:   $repo/actions"
    Write-Host "Release: $repo/releases/tag/$tag  (appears once the build passes; macOS runners can take a while)"
    Write-Host 'Check it lists all seven files before sharing it.'
}

switch ($Command) {
    'build' { Build }
    'test' { Step 'Tests'; Run { cargo test --release } }
    'start' {
        if (-not (Test-Path $exe)) { Build }
        Stop-Running
        Step "Starting $exe $Rest"
        & $exe @Rest
    }
    'dev' {
        # phone app served from web\ so edits show after a reload, no rebuild needed
        Stop-Running
        $env:SOFA_WEB_DIR = Join-Path $root 'web'
        Step 'Dev server (web\ served from disk)'
        Run { cargo run --release -- @Rest }
    }
    'stop' { Stop-Running; Write-Host 'Stopped.' }
    'apk' { Apk }
    'all' { & (Join-Path $root 'release.ps1') @Rest; if ($LASTEXITCODE) { exit $LASTEXITCODE } }
    'release' { Release ($Rest | Select-Object -First 1) }
    'version' { Version }
    default {
        Write-Host @"
Sofa Remote: cli <command>

  build              build target\release\sofa-remote.exe (gamepad driver bundled when available)
  start [args]       stop any running copy, then run the built exe (builds it first if missing)
                     e.g. cli start --port 8765 --name "Living room"
  dev [args]         run from source with the phone app served from web\ (reload to see edits)
  stop               stop a running sofa-remote
  test               cargo test --release
  apk                build the Android APK into release\
  all [-Skip ...]    build everything into release\ (release.ps1)
  version            print the current version
  release X.Y.Z      bump the version, test, build the APK, then (after you type yes) commit,
                     push main and push tag vX.Y.Z so GitHub Actions publishes the release
"@
    }
}
