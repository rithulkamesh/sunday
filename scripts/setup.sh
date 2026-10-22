#!/usr/bin/env bash
# One-time setup, safe to rerun: models, OCR helper, optional router, JS deps, and a check that a model provider is up.
# Usage: scripts/setup.sh [--no-router] [--vision]
set -euo pipefail
cd "$(dirname "$0")/.."

router=1 vision=0
for a in "$@"; do
  case "$a" in --no-router) router=0 ;; --vision) vision=1 ;; *) echo "unknown option $a" >&2; exit 2 ;; esac
done

missing=0
need() { command -v "$1" >/dev/null || { echo "missing: $1 ($2)"; missing=1; }; }
need bun "https://bun.sh"
need cargo "https://rustup.rs"
[ "$(uname -s)" = Darwin ] || echo "note: only macOS is supported for now"
[ "$missing" = 0 ] || exit 1

echo "== speech models"; scripts/fetch-models.sh
echo "== OCR helper";   scripts/build-ocr.sh
if [ "$router" = 1 ]; then
  echo "== routing model (optional)"
  scripts/fetch-router.sh || echo "skipped: Sunday works without it (rules route alone). Retry with scripts/fetch-router.sh"
fi
echo "== JS dependencies"; bun install

# A provider is where chat runs. Any one will do; Settings picks it.
up() { curl -fsS -m 2 "$1" >/dev/null 2>&1; }
found=0
if up http://localhost:11434/api/tags; then
  found=1; echo "provider: Ollama is running"
  ollama pull qwen3:4b-instruct
  ollama pull nomic-embed-text-v2-moe
  [ "$vision" = 1 ] && ollama pull qwen3-vl:4b-instruct
fi
if up http://localhost:1234/v1/models; then found=1; echo "provider: LM Studio is running (Settings -> Models -> LM Studio)"; fi
if [ "$found" = 0 ]; then
  echo "provider: none running. Start Ollama (https://ollama.com) or LM Studio's server, or use MLX:"
  echo "  scripts/fetch-mlx-model.sh mlx-community/Qwen3-4B-Instruct-2507-4bit"
fi

echo "done. Run: bun run tauri dev"
