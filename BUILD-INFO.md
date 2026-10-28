# Build information — 0.1.9

Built locally on **2026-10-28**, Europe/Rome, on Windows x64.
These are local build records, not a published GitHub Release.

## Versions and components

| Component | Version / configuration |
| --- | --- |
| PC server | 0.1.9; Rust 2024 edition; default WebRTC feature enabled |
| Android application | `app.sofaremote`; version name 0.1.9; version code 10 |
| Android compatibility | Minimum SDK 26 (Android 8); target SDK 34; compile SDK 35 |
| Phone interface | Embedded in the PC executable; loaded by the Android WebView |
| Windows gamepads | ViGEmBus 1.22.0; installer bundled in the Windows executable |
| ViGEm client | Vendored 0.1.4 with the fixes described in `vendor/vigem-client/SOFA-PATCH.md` |
| Android QR scanner | ZXing Android Embedded 4.3.0; camera and decoder bundled in the APK |
| Rust / Cargo | 1.93.1 / 1.93.1 |
| Java | Microsoft OpenJDK 17.0.20.1 |
| Android build | Gradle 8.14.3; Android Gradle Plugin 8.10.1; Kotlin 2.1.21 |
| JavaScript test runtime | Node.js 25.9.0 |

The Windows startup message and `sofa-remote --version` identify the server version.
Update both the PC executable and Android APK for the new scanner and offline controls.

## Local artifacts

The Windows executable and APK were rebuilt after updating their version metadata.
SHA-256 values below identify these exact local files; later CI builds may produce different hashes.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `release/sofa-remote-windows-x64.exe` | 11926016 | `46208ff7dcae2e747043cdcaa14028a8a4e045e04f0cff53829e35871458118e` |
| `release/sofa-remote-android.apk` | 2418033 | `bb58f0abf18d61eb5ac874922bb7403e0ff7c15d631f09e402f134bf501e428c` |
| `vendor/ViGEmBus_1.22.0_x64_x86_arm64.exe` | 6278576 | `89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a` |

The APK uses the project's existing release signing key. APK Signature Scheme v2 verification
passed with one signer. The driver installer matches the pinned publisher file hash.
Executables, APKs, signing keys, access tokens, and saved layouts are excluded from commits.

## Build and verification commands

Run from the repository root in PowerShell, with the Android SDK and Java 17 configured.
`vendor/fetch.ps1` obtains the pinned driver installer if it is absent.

```powershell
$env:SOFA_REQUIRE_DRIVER = '1'
cargo test --release
cargo build --release
node --test tests/gamepad.test.cjs tests/connections.test.cjs

Push-Location android
.\gradlew.bat assembleRelease
Pop-Location

Copy-Item target/release/sofa-remote.exe release/sofa-remote-windows-x64.exe -Force
.\release\sofa-remote-windows-x64.exe --version
```

Gradle copies the signed APK to `release/sofa-remote-android.apk`. With Android SDK Build Tools
35.0.0, verify the APK using `apksigner.bat verify --verbose` and inspect its version metadata
using `aapt.exe dump badging`.

## Verification results

- `cargo test --release`: 11 passed; the hardware-dependent controller test is ignored by default.
- JavaScript regressions: 8 passed, covering controller recovery, stale replies, disconnect
  cancellation, Android confirmation routing, saved connection removal, and stopped reconnects.
- Windows release build: passed with `SOFA_REQUIRE_DRIVER=1`, which requires the bundled installer.
- Both `--version` and `-V`: returned `sofa-remote 0.1.9` and exited successfully.
- Android release build and release lint: passed. APK metadata reports version 0.1.9, code 10.
- APK signature verification: passed.

Functional verification earlier in this change, before the version-only rebuild:

- Native Windows test passed with two independent virtual controllers and 128 successive input
  changes; multiple phone connections appeared as separate entries in the browser Gamepad API.
- The Android APK installed and launched in the emulator. Camera permission denial, scanner
  opening, and cancellation were checked. A complete camera-to-connection QR scan was not verified.
- Native offline confirmation returned to setup and stopped the background control service.
  Browser checks verified the scanner and offline buttons invoke the Android bridge.

## Remaining release builds

Linux and macOS binaries were not rebuilt locally for 0.1.9. Any older Linux binary already in
`release/` is not part of the 0.1.9 build record. The complete seven-file release set must be
built and checked by `.github/workflows/release.yml` before publishing:

| Required release asset | Local 0.1.9 status |
| --- | --- |
| `sofa-remote-windows-x64.exe` | Rebuilt; driver bundled |
| `sofa-remote-linux-x64` | Requires CI build |
| `sofa-remote-linux-arm64` | Requires CI build |
| `sofa-remote-macos-arm64` | Requires CI build |
| `sofa-remote-macos-x64` | Requires CI build |
| `sofa-remote-android.apk` | Rebuilt; signed and verified |
| `ViGEmBus_1.22.0_x64_x86_arm64.exe` | Pinned installer verified in `vendor/` |

See [CHANGELOG.md](CHANGELOG.md) for the user-visible changes and [AGENTS.md](AGENTS.md) for
the complete release checklist.
