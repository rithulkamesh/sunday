#!/usr/bin/env bash
# Download an MLX model from Hugging Face for the "MLX (on this Mac)" provider.
#   scripts/fetch-mlx-model.sh mlx-community/Qwen3-4B-Instruct-2507-4bit
# Lands in ~/.cache/sunday/models/mlx/<owner>--<name>. The app never downloads anything itself.
set -euo pipefail
repo="${1:?usage: fetch-mlx-model.sh <owner/name>, e.g. mlx-community/Qwen3-4B-Instruct-2507-4bit}"
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "not a Hugging Face repo id: $repo" >&2; exit 1; }
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/mlx/${repo/\//--}"
mkdir -p "$dir" && cd "$dir"
for f in $(curl -fsSL "https://huggingface.co/api/models/$repo" | python3 -c 'import json,sys; [print(s["rfilename"]) for s in json.load(sys.stdin)["siblings"]]'); do
  case "$f" in .*|*/*|*.md) continue ;; esac
  [ -f "$f" ] || curl -fSL --progress-bar -o "$f.partial" "https://huggingface.co/$repo/resolve/main/$f" && mv "$f.partial" "$f"
done
echo "model in $dir"
