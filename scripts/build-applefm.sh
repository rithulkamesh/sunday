#!/usr/bin/env bash
# Optional: compile the helper that lets Sunday chat with Apple's on-device model. Needs macOS 26 and Xcode.
set -euo pipefail
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/../bin"
mkdir -p "$dir"
swiftc -O "$(dirname "$0")/applefm.swift" -o "$dir/applefm"
echo "built $dir/applefm"
