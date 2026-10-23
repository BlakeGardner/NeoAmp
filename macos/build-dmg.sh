#!/usr/bin/env bash
# Puts the NeoAmp.app that macos/build-app.sh built for Apple silicon (arm64) or Intel (x86_64)
# Macs in a disk image whose window shows it beside a link to Applications to drag it onto:
# target/macos/<arch>/neoamp-v<version>-macos-<arch>.dmg.
#
#   macos/build-dmg.sh [arm64|x86_64]
#
# Without an argument it packs the app for the Mac it runs on. On its own it leaves the app
# signed ad hoc, so macOS asks before opening it the first time. Given these, it signs the app
# and the disk image with a Developer ID, has Apple notarize the disk image and staples the
# ticket to it, so it opens like any other app downloaded from the web:
#
#   SIGN_IDENTITY     a "Developer ID Application" certificate in the keychain, by its name or
#                     its SHA-1 hash, as `security find-identity -v -p codesigning` lists it
#   NOTARY_PROFILE    notary service credentials saved with `xcrun notarytool store-credentials`
#   NOTARY_KEYCHAIN   the keychain they were saved in, when it is not the login keychain
#
# See "Signing and notarizing" in docs/DEVELOPMENT.md. The Release workflow runs this for each
# GitHub release, with all three (.github/workflows/macos.yml).

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Darwin ]]; then
    echo "build-dmg.sh: it makes a macOS disk image, so it runs only on a Mac" >&2
    exit 1
fi
arch=${1:-$(uname -m)}
case $arch in
    arm64 | x86_64) ;;
    *)
        echo "usage: macos/build-dmg.sh [arm64|x86_64]" >&2
        exit 1
        ;;
esac
sign=${SIGN_IDENTITY:-}
profile=${NOTARY_PROFILE:-}
if [[ -n $profile && -z $sign ]]; then
    echo "build-dmg.sh: Apple notarizes only what is signed with a Developer ID: set SIGN_IDENTITY too" >&2
    exit 1
fi

out=target/macos/$arch
app=$out/NeoAmp.app
if [[ ! -d $app ]]; then
    echo "build-dmg.sh: there is no $app: build it first with 'macos/build-app.sh $arch'" >&2
    exit 1
fi
version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")
# What the disk image holds.
stage=$out/stage
dmg=$out/neoamp-v$version-macos-$arch.dmg

if [[ -n $sign ]]; then
    # The hardened runtime and a timestamp from Apple's server are what notarization asks for.
    codesign --force --sign "$sign" --options runtime --timestamp "$app"
    codesign --verify --strict --verbose=2 "$app"
fi

rm -rf "$stage" "$dmg"
mkdir -p "$stage"
ditto "$app" "$stage/NeoAmp.app"
ln -s /Applications "$stage/Applications"
# hdiutil now and then fails with "Resource busy" on GitHub's Mac runners: wait and try again.
sync
for attempt in 1 2 3 4 5; do
    if hdiutil create -volname NeoAmp -srcfolder "$stage" -format UDZO -ov "$dmg"; then
        break
    fi
    if ((attempt == 5)); then
        echo "build-dmg.sh: hdiutil could not make the disk image" >&2
        exit 1
    fi
    sleep $((attempt * 10))
done

if [[ -n $sign ]]; then
    codesign --sign "$sign" --timestamp "$dmg"
fi

if [[ -n $profile ]]; then
    notary=(--keychain-profile "$profile")
    if [[ -n ${NOTARY_KEYCHAIN:-} ]]; then
        notary+=(--keychain "$NOTARY_KEYCHAIN")
    fi
    # Apple checks the disk image and everything in it, usually within a few minutes; the
    # property list notarytool prints at the end says whether it was accepted.
    result=$(xcrun notarytool submit "$dmg" "${notary[@]}" --wait --output-format plist) || true
    status=$(plutil -extract status raw - <<<"$result" 2>/dev/null) || status=
    if [[ $status != Accepted ]]; then
        echo "$result" >&2
        echo "build-dmg.sh: Apple did not notarize $dmg${status:+ ($status)}" >&2
        if id=$(plutil -extract id raw - <<<"$result" 2>/dev/null); then
            echo "build-dmg.sh: the notary service's log says why:" >&2
            xcrun notarytool log "$id" "${notary[@]}" >&2 || true
        fi
        exit 1
    fi
    # The ticket, stapled to the disk image, lets macOS check it without asking Apple. Apple
    # now and then takes a little while to hand the ticket out: wait and try again.
    for attempt in 1 2 3 4 5; do
        if xcrun stapler staple "$dmg"; then
            break
        fi
        if ((attempt == 5)); then
            echo "build-dmg.sh: stapler could not staple the notary ticket to $dmg" >&2
            exit 1
        fi
        sleep $((attempt * 15))
    done
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
elif [[ -n $sign ]]; then
    echo "build-dmg.sh: signed, but not notarized without NOTARY_PROFILE, so macOS still asks before opening it" >&2
fi
echo "Built $dmg"
