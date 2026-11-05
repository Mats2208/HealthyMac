#!/bin/zsh
# Launches the debug app, captures its window to a PNG and quits it.
# Usage: scripts/screenshot.sh <output.png> [seconds-to-wait] [any args for the app]
set -u
out="$1"; wait_secs="${2:-3}"; shift; [ $# -gt 0 ] && shift
root="${0:A:h:h}"

"$root/target/debug/HealthyMac" "$@" &
pid=$!
trap 'kill $pid 2>/dev/null' EXIT
sleep "$wait_secs"

window_id=$(swift - "$pid" <<'SWIFT'
import CoreGraphics
import Foundation

let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
let window = windows.first {
    ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0
}
print((window?[kCGWindowNumber as String] as? Int).map(String.init) ?? "")
SWIFT
)

if [ -z "$window_id" ]; then
  echo "no se encontró la ventana de la app (pid $pid)" >&2
  exit 1
fi
screencapture -x -o -l"$window_id" "$out" && echo "$out"
