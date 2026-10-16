#!/usr/bin/env bash
# One-time: compile the screen-text reader used to point at exact lines. Needs Xcode command line tools.
set -euo pipefail
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/../bin"
mkdir -p "$dir"
swiftc -O "$(dirname "$0")/ocr.swift" -o "$dir/ocr"
echo "built $dir/ocr"
