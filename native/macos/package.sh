#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

binary=${1:-target/release/fero}
case "$binary" in /*) ;; *) binary="$PWD/$binary" ;; esac
arch=$(lipo -archs "$binary")
case "$arch" in
    x86_64|arm64) ;;
    *) echo "Expected a native Intel or Apple Silicon build, got: $arch" >&2; exit 1 ;;
esac
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' native/macos/Info.plist)
stage=$(mktemp -d)
trap 'rm -r "$stage"' EXIT
app="$stage/Fero.app/Contents"
mkdir -p "$app/MacOS" "$app/Frameworks" "$app/Resources" dist
cp native/macos/Info.plist "$app/Info.plist"
cp native/macos/Fero.icns "$app/Resources/"
cp "$binary" "$app/MacOS/fero"
strip -x "$app/MacOS/fero"

bundle_dependencies() {
    local source=$1 dest=$2 prefix=$3 dependency filename bundled
    while IFS= read -r dependency; do
        case "$dependency" in
            /System/Library/*|/usr/lib/*) continue ;;
            /*.dylib) ;;
            *) echo "Unsupported dependency in $source: $dependency" >&2; return 1 ;;
        esac
        filename=${dependency##*/}
        bundled="$app/Frameworks/$filename"
        # Compare before editing any copies, so different libraries sharing a
        # basename cannot silently replace one another.
        if [[ -e "$stage/$filename" ]]; then
            cmp -s "$dependency" "$stage/$filename" || {
                echo "Conflicting dylibs: $dependency" >&2; return 1;
            }
        else
            cp "$dependency" "$stage/$filename"
            cp "$dependency" "$bundled"
            lipo "$bundled" -verify_arch "$arch"
            chmod u+w "$bundled"
            install_name_tool -id "@loader_path/$filename" "$bundled"
            bundle_dependencies "$dependency" "$bundled" '@loader_path'
        fi
        install_name_tool -change "$dependency" "$prefix/$filename" "$dest"
    done < <(otool -L "$source" | awk 'NR > 1 {sub(/^[[:space:]]+/, ""); sub(/ \(compatibility version.*/, ""); print}' | {
        if [[ "$prefix" == '@loader_path' ]]; then tail -n +2; else cat; fi
    })
}

bundle_dependencies "$binary" "$app/MacOS/fero" '@executable_path/../Frameworks'
identity=${FERO_CODESIGN_IDENTITY:--}
sign_options=()
if [[ "$identity" != '-' ]]; then
    sign_options=(--options runtime --timestamp)
fi
for dylib in "$app"/Frameworks/*.dylib; do
    [[ ! -f "$dylib" ]] || codesign --force ${sign_options[@]+"${sign_options[@]}"} --sign "$identity" "$dylib"
done
codesign --force ${sign_options[@]+"${sign_options[@]}"} --sign "$identity" "$stage/Fero.app"
codesign --verify --deep --strict "$stage/Fero.app"

# Keep the original dylibs used for collision checks out of the disk image.
mkdir "$stage/image"
mv "$stage/Fero.app" "$stage/image/"
ln -s /Applications "$stage/image/Applications"
hdiutil create -size 256m -volname Fero -srcfolder "$stage/image" -format UDZO "$stage/Fero.dmg"
mv "$stage/Fero.dmg" "dist/Fero-$version-macos-$arch.dmg"
