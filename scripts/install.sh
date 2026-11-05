#!/bin/sh
# Installs the latest HealthyMac release into /Applications.
#
# The app is signed ad hoc, not notarized. A browser would mark the download as quarantined and
# Gatekeeper would block it; curl doesn't, so this path needs no extra steps.
set -eu

url="https://github.com/fernandevdaza/HealthyMac/releases/latest/download/HealthyMac-mac-arm64.dmg"

if [ "$(uname -m)" != "arm64" ]; then
    echo "This release is for Apple Silicon. On an Intel Mac, build it from source." >&2
    exit 1
fi
if pgrep -x HealthyMac >/dev/null; then
    echo "HealthyMac is running. Quit it and try again." >&2
    exit 1
fi

dest=/Applications
if [ ! -w "$dest" ]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
fi

work="$(mktemp -d)"
mount="$work/mount"
cleanup() {
    hdiutil detach "$mount" -quiet 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

curl -fL --progress-bar "$url" -o "$work/HealthyMac.dmg"
hdiutil attach "$work/HealthyMac.dmg" -quiet -nobrowse -readonly -mountpoint "$mount"
rm -rf "$dest/HealthyMac.app"
ditto "$mount/HealthyMac.app" "$dest/HealthyMac.app"

echo "Installed in $dest/HealthyMac.app"
echo "Open it with: open \"$dest/HealthyMac.app\""
