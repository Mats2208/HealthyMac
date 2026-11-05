#!/bin/zsh
# Renders docs/banner.html to docs/banner.png and docs/banner.es.png with headless Google Chrome.
set -eu
root="${0:A:h:h}"
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
profile="$(mktemp -d)"
trap 'rm -rf "$profile"' EXIT

render() {
    "$chrome" --headless=new --user-data-dir="$profile" --no-first-run --hide-scrollbars \
        --force-device-scale-factor=2 --window-size=1280,480 \
        --screenshot="$1" "file://$root/docs/banner.html?lang=$2" 2>/dev/null
}

render "$root/docs/banner.png" en
render "$root/docs/banner.es.png" es
