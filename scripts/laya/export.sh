#!/usr/bin/env bash
# Export the fine-tuned checkpoint to ONNX and install it where the app looks: ~/.cache/sunday/models/laya-routing.
#   LAYA_REPO=/path/to/clone-of-NandhaKishorM/laya scripts/laya/export.sh laya-work/model
# Needs the Python environment from scripts/laya/README.md. The export is verified against the PyTorch model.
set -euo pipefail
ckpt="${1:?usage: export.sh <checkpoint dir>}"
repo="${LAYA_REPO:?set LAYA_REPO to a clone of https://github.com/NandhaKishorM/laya}"
tmp="$(mktemp -d)"
python "$repo/laya-rust/tools/export_onnx.py" "$ckpt" --out "$tmp/laya-routing" --verify
dest="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/laya-routing"
rm -rf "$dest" && mkdir -p "$(dirname "$dest")" && cp -R "$tmp/laya-routing" "$dest"
echo "installed $dest"
