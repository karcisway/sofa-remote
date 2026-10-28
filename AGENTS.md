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

## Project

- PC server: Rust (`src/`). `cargo build --release`, `cargo test --release`.
- Phone app: plain HTML/CSS/JS in `web/`, embedded into the binary at build time.
  `SOFA_WEB_DIR=web` serves it from disk while editing.
- Android app: `android/` (Kotlin, WebView around the phone app). `./gradlew assembleRelease`
  copies the APK to `release/`.
- Everything into `release/`: `release.ps1` (Windows) or `release.sh`.
- Landing page: `docs/` (GitHub Pages). Brand assets and guide: `brand/`.
