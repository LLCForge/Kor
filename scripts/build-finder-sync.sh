#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Build Kor's Finder Sync extension (§25.2, ADR 0052) as a universal, signed .appex.
#
#   scripts/build-finder-sync.sh <out-dir> <version> [signing identity]
#
# Writes <out-dir>/KorFinderSync.appex. The identity defaults to "-" (ad-hoc), matching the
# unsigned desktop build; release CI passes the Developer ID used for Kor.app, because macOS only
# loads an extension signed by the same team as its app. Runs its own unit tests first.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: $0 <out-dir> <version> [signing identity]" >&2
  exit 2
fi
out=$1
version=$2
identity=${3:--}
here=$(cd "$(dirname "$0")/.." && pwd)
src="$here/frontend/src-tauri/macos/KorFinderSync"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Pure menu logic tests (same rules as kor-shellext/src/menu.rs).
xcrun swiftc -O "$src/Menu.swift" "$src/Tests/main.swift" -o "$tmp/menu-tests"
"$tmp/menu-tests"

sdk=$(xcrun --sdk macosx --show-sdk-path)
for arch in arm64 x86_64; do
  # App extensions start in NSExtensionMain (no `main`); -application-extension limits the API.
  xcrun swiftc -O -sdk "$sdk" -target "$arch-apple-macos10.15" -module-name KorFinderSync \
    -application-extension -framework Cocoa -framework FinderSync \
    -Xlinker -e -Xlinker _NSExtensionMain \
    "$src/Menu.swift" "$src/FinderSync.swift" -o "$tmp/KorFinderSync-$arch"
done

appex="$out/KorFinderSync.appex"
rm -rf "$appex"
mkdir -p "$appex/Contents/MacOS"
lipo -create -output "$appex/Contents/MacOS/KorFinderSync" "$tmp/KorFinderSync-arm64" "$tmp/KorFinderSync-x86_64"
cp "$src/Info.plist" "$appex/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" -c "Set :CFBundleVersion $version" \
  "$appex/Contents/Info.plist"
plutil -lint "$appex/Contents/Info.plist"

if [ "$identity" = "-" ]; then
  codesign --force --sign - --entitlements "$src/KorFinderSync.entitlements" "$appex"
else
  codesign --force --options runtime --timestamp --sign "$identity" \
    --entitlements "$src/KorFinderSync.entitlements" "$appex"
fi
codesign --verify --strict --verbose=2 "$appex"
echo "built $appex"
