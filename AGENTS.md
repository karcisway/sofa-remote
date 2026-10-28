# Notes for coding agents

## Commits

- One commit per feature or fix. Each commit should do one thing that can be described in one
  sentence, build on its own, and be revertable without taking unrelated work with it.
- When several features were worked on together, split them before committing (stage by file,
  or by hunk with `git add -p`) and commit them in a sensible order: shared groundwork first,
  then each feature, then docs that span them.
- A feature's README/doc changes, tests and assets go in the same commit as the feature.
- Subject line: imperative, under ~70 characters, says what changed for the user
  ("Add pink Bubblegum theme"), not how ("Update app.css"). Use the body for the why and
  anything non-obvious.
- Don't mix refactors or formatting with behaviour changes; give them their own commit.
- Never commit built binaries, `release/` contents, keys (`.remote_token`, `*.keystore`) or
  `layouts.json`.

## Releases

Every GitHub Release must let someone download everything and install it correctly, with
nothing fetched from elsewhere. It carries all seven files, rebuilt from the tag even when a part
didn't change:

| File | What it must be |
| --- | --- |
| `sofa-remote-windows-x64.exe` | With the gamepad driver installer built in (about 11 MB; `SOFA_REQUIRE_DRIVER=1` makes the build fail without it) |
| `sofa-remote-linux-x64`, `sofa-remote-linux-arm64` | |
| `sofa-remote-macos-arm64`, `sofa-remote-macos-x64` | |
| `sofa-remote-android.apk` | |
| `ViGEmBus_1.22.0_x64_x86_arm64.exe` | The gamepad driver installer, SHA-256 checked |

- `.github/workflows/release.yml` builds them when a tag `vX.Y.Z` is pushed. Its last step checks
  the whole set (and the bundled driver) and creates no release when anything is missing. A new
  download goes into that workflow's build, its check and this table.
- Never publish a release with files missing, and never upload or replace single files by hand.
- To release: run `cargo test --release` and build the APK locally first, push main, then push a
  new tag (never move or re-push an existing one).
- A release is done only when it's been checked: the tag's Actions run passed, and the release
  page lists all seven files with sensible sizes. Tell the user the release link only then.
  macOS runners can queue for a while, so this can take some minutes.
- If the run fails, find the failing job and fix the cause, then release under a new tag.
  Report the failure to the user; don't say a release is out when it isn't.
- Before releasing a change to how something installs or starts (the driver installer, the
  Android app), try that path for real once, or tell the user plainly it wasn't tried.

## Project

- PC server: Rust (`src/`). `cargo build --release`, `cargo test --release`.
- Phone app: plain HTML/CSS/JS in `web/`, embedded into the binary at build time.
  `SOFA_WEB_DIR=web` serves it from disk while editing.
- Android app: `android/` (Kotlin, WebView around the phone app). `./gradlew assembleRelease`
  copies the APK to `release/`.
- Everything into `release/`: `release.ps1` (Windows) or `release.sh`.
- Landing page: `docs/` (GitHub Pages). Brand assets and guide: `brand/`.
