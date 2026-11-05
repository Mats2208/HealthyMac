#!/bin/zsh
# Regenerates crates/hm-app/assets/AppIcon.icns from scripts/make_icon.swift.
set -eu
root="${0:A:h:h}"
work="$(mktemp -d)"
trap 'rm -r "$work"' EXIT

swift "$root/scripts/make_icon.swift" "$work/icon.png"

set_dir="$work/AppIcon.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$work/icon.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$work/icon.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done

mkdir -p "$root/crates/hm-app/assets"
iconutil -c icns "$set_dir" -o "$root/crates/hm-app/assets/AppIcon.icns"
cp "$work/icon.png" "$root/crates/hm-app/assets/AppIcon.png"
echo "$root/crates/hm-app/assets/AppIcon.icns"
