#!/usr/bin/env bash
# Builds NeoAmp.app for Apple silicon (arm64) or Intel (x86_64) Macs, signed ad hoc:
# target/macos/<arch>/NeoAmp.app. macos/build-dmg.sh then puts it in a disk image, signing it
# with a Developer ID first when it is given one.
#
#   macos/build-app.sh [arm64|x86_64]
#
# Without an argument it builds for the Mac it runs on. It runs on a Mac, from anywhere in this
# repository, and needs Rust's target for that kind of Mac, and resvg, which draws the icon:
#
#   rustup target add aarch64-apple-darwin    (or x86_64-apple-darwin)
#   cargo install --locked resvg
#
# The Release workflow runs this for each GitHub release (.github/workflows/macos.yml).

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Darwin ]]; then
    echo "build-app.sh: it builds a macOS app, so it runs only on a Mac" >&2
    exit 1
fi
arch=${1:-$(uname -m)}
case $arch in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *)
        echo "usage: macos/build-app.sh [arm64|x86_64]" >&2
        exit 1
        ;;
esac
if ! command -v resvg >/dev/null; then
    echo "build-app.sh: resvg draws the icon: install it with 'cargo install --locked resvg'" >&2
    exit 1
fi

plist=macos/Info.plist
icon=resources/icons/neoamp-macos.svg
# "path+file:///…/NeoAmp#neoamp@0.2.0", or "…#0.2.0" when the folder is named neoamp.
pkgid=$(cargo pkgid)
version=${pkgid##*[#@]}
# The compiler builds for the oldest macOS the Info.plist names.
MACOSX_DEPLOYMENT_TARGET=$(/usr/libexec/PlistBuddy -c "Print :LSMinimumSystemVersion" "$plist")
export MACOSX_DEPLOYMENT_TARGET

out=target/macos/$arch
app=$out/NeoAmp.app
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cargo build --release --locked --target "$target"
cp "target/$target/release/neoamp" "$app/Contents/MacOS/NeoAmp"

cp "$plist" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy \
    -c "Set :CFBundleShortVersionString $version" \
    -c "Set :CFBundleVersion $version" \
    "$app/Contents/Info.plist"

# The icon, drawn at each size macOS uses.
iconset=$out/NeoAmp.iconset
mkdir "$iconset"
for size in 16 32 128 256 512; do
    resvg -w "$size" -h "$size" "$icon" "$iconset/icon_${size}x${size}.png"
    resvg -w $((size * 2)) -h $((size * 2)) "$icon" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns -o "$app/Contents/Resources/NeoAmp.icns" "$iconset"

# Apple silicon runs only signed code. An ad hoc signature, made here, needs no Apple account;
# it also seals the Info.plist and the icon to the program.
codesign --force --sign - "$app"
codesign --verify --strict --verbose=2 "$app"
echo "Built $app"
