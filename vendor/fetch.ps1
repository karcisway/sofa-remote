# Downloads the ViGEmBus gamepad driver installer (BSD-3-Clause, LICENSE-ViGEmBus.txt) into
# vendor/, checked against its known SHA-256. The Windows build then bundles it (see build.rs).
$ErrorActionPreference = 'Stop'
$name = 'ViGEmBus_1.22.0_x64_x86_arm64.exe'
$sha256 = '89220A7865076B342892F98865F3499FB7C4CFD673159E89D352C360FD014C6A'
$file = Join-Path $PSScriptRoot $name
if ((Test-Path $file) -and (Get-FileHash $file).Hash -eq $sha256) { Write-Host "vendor/$name already there"; exit 0 }
[Net.ServicePointManager]::SecurityProtocol = 'Tls12'
Invoke-WebRequest -UseBasicParsing -OutFile $file `
    -Uri "https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/$name"
if ((Get-FileHash $file).Hash -ne $sha256) { Remove-Item $file; throw "$name has an unexpected SHA-256" }
Write-Host "vendor/$name downloaded"
