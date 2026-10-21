#!/usr/bin/env bash
# Build the routing sidecar (tools/laya-router) and install it where the app looks: ~/.cache/sunday/bin/laya-router.
# Nothing is downloaded by the app itself; this fetches the laya-onnx crate and builds it once.
set -euo pipefail
cd "$(dirname "$0")/../tools/laya-router"
cargo build --release
dest="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/../bin"
mkdir -p "$dest" && cp target/release/laya-router "$dest/"
echo "installed $dest/laya-router"
