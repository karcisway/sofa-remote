#!/bin/sh
# Builds this machine's binary and the Android APK into release/.
#   ./release.sh            build all
#   SKIP=android ./release.sh
set -e
cd "$(dirname "$0")"
mkdir -p release

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) name=sofa-remote-linux-x64 ;;
  Linux-aarch64) name=sofa-remote-linux-arm64 ;;
  Darwin-arm64) name=sofa-remote-macos-arm64 ;;
  Darwin-x86_64) name=sofa-remote-macos-x64 ;;
  *) name=sofa-remote ;;
esac

echo "== $name"
cargo build --release
cp target/release/sofa-remote "release/$name"

case "$SKIP" in *android*) ;; *)
  echo "== Android"
  (cd android && ./gradlew assembleRelease)   # copies the APK to release/ by itself
esac

ls -lh release
