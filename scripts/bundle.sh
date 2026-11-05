#!/bin/zsh
# Wraps the built binary in target/<profile>/HealthyMac.app so macOS treats it as a real app, which
# gives it a stable identity for permissions such as Full Disk Access.
# Usage: scripts/bundle.sh [debug|release]
set -eu
profile="${1:-debug}"
root="${0:A:h:h}"
app="$root/target/$profile/HealthyMac.app"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)"

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/$profile/healthymac-app" "$app/Contents/MacOS/HealthyMac"
cp "$root/crates/hm-app/assets/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>HealthyMac</string>
  <key>CFBundleDisplayName</key><string>HealthyMac</string>
  <key>CFBundleIdentifier</key><string>dev.healthymac.app</string>
  <key>CFBundleExecutable</key><string>HealthyMac</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature: required on Apple Silicon, and lets TCC remember permissions for this build.
codesign --force --sign - "$app" >/dev/null 2>&1
echo "$app"
