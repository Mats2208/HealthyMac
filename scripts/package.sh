#!/bin/zsh
# Builds the release app and wraps it in target/release/HealthyMac-mac-<arch>.dmg, ready to publish.
# The app is signed ad hoc: enough for Apple Silicon to run it, but it is not notarized.
set -eu
root="${0:A:h:h}"
cd "$root"

cargo build --release -p hm-app
scripts/bundle.sh release >/dev/null
codesign --verify --deep --strict target/release/HealthyMac.app

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
ditto target/release/HealthyMac.app "$stage/HealthyMac.app"
ln -s /Applications "$stage/Applications"

dmg="$root/target/release/HealthyMac-mac-$(uname -m).dmg"
hdiutil create -quiet -ov -volname HealthyMac -srcfolder "$stage" -format UDZO "$dmg"
echo "$dmg"
