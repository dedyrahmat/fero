#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."

binary=${1:-target/release/fero}
linuxdeploy=${LINUXDEPLOY:-linuxdeploy}
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)
if [ "$(uname -m)" != x86_64 ] || ! LC_ALL=C readelf -h "$binary" | grep -q 'Machine:.*X86-64'; then
    echo 'Expected an x86-64 Linux build' >&2
    exit 1
fi
stage=$(mktemp -d)
trap 'rm -r "$stage"' EXIT
mkdir -p "$stage/AppDir/usr/bin" dist
install -m 755 "$binary" "$stage/AppDir/usr/bin/fero"
strip --strip-debug "$stage/AppDir/usr/bin/fero"
install -m 644 design/brand/app-icon.png "$stage/dev.fero.desktop.png"
OUTPUT="$stage/Fero.AppImage" \
    "$linuxdeploy" --appdir "$stage/AppDir" \
    --executable "$stage/AppDir/usr/bin/fero" \
    --desktop-file native/linux/dev.fero.desktop.desktop \
    --icon-file "$stage/dev.fero.desktop.png" --output appimage
test -s "$stage/Fero.AppImage"
chmod +x "$stage/Fero.AppImage"
mv "$stage/Fero.AppImage" "dist/Fero-$version-linux-x86_64.AppImage"
