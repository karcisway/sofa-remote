Built binaries land here (this folder is not committed):

- `.\release.ps1` (Windows) or `./release.sh` (Linux/macOS) builds everything.
- `cd android && ./gradlew assembleRelease` copies `sofa-remote-android.apk` here by itself
  (`assembleDebug` gives `sofa-remote-android-debug.apk`).

Ready-made downloads are on the GitHub Releases page.
