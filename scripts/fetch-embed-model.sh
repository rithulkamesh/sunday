#!/usr/bin/env bash
# Download the on-device embedding model (all-MiniLM-L6-v2, about 90 MB) for memory search without Ollama.
# Lands in ~/.cache/sunday/models/embed/all-MiniLM-L6-v2. The app never downloads anything itself.
set -euo pipefail
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/embed/all-MiniLM-L6-v2"
base=https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main
mkdir -p "$dir" && cd "$dir"
for f in config.json tokenizer.json model.safetensors; do
  [ -f "$f" ] || { curl -fSL --progress-bar -o "$f.partial" "$base/$f" && mv "$f.partial" "$f"; }
done
echo "model in $dir"
