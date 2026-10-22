#!/usr/bin/env bash
# Maintainers: publish the routing weights to Hugging Face and pin them in git. Needs the `hf` CLI
# (uv tool install huggingface_hub) and HF_TOKEN with write access, in the environment or in a git-ignored .env.
# A new repo is created private; make it public in its Hub settings when you're ready. Usage: scripts/laya/publish.sh vN
# Then push the router-vN tag so CI builds and attaches the program (.github/workflows/release-router.yml).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
export PATH="$HOME/.local/bin:$PATH"
if [ -f "$here/../../.env" ]; then set -a; . "$here/../../.env"; set +a; fi
: "${HF_TOKEN:?set HF_TOKEN (write access) in the environment or .env}"
tag="router-${1:?usage: publish.sh vN (e.g. v1)}"
models="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}/laya-routing"
repo="${SUNDAY_ROUTER_REPO:-rithulkamesh/sunday-laya-router}"
files=(model.onnx model.onnx.data rl_agent_config.json tokenizer/tokenizer.json tokenizer/tokenizer_config.json)

# Only the inference files go up (not fixtures.npz). The card is the repo's README.
stage="$(mktemp -d)"; trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/tokenizer"
for f in "${files[@]}"; do ln "$models/$f" "$stage/$f" 2>/dev/null || cp "$models/$f" "$stage/$f"; done
cp "$here/MODEL_CARD.md" "$stage/README.md"
cp "$here/../../LICENSE" "$stage/LICENSE"

hf upload "$repo" "$stage" . --repo-type model --private --commit-message "Sunday router $tag"
rev="$(curl -fsS -H "Authorization: Bearer $HF_TOKEN" "https://huggingface.co/api/models/$repo" | sed -n 's/.*"sha":"\([0-9a-f]\{40\}\)".*/\1/p' | head -1)"
[ -n "$rev" ] || { echo "could not read the commit sha of $repo" >&2; exit 1; }

{
  echo "# rev $rev"
  echo "# release $tag"
  for f in "${files[@]}"; do echo "$(shasum -a 256 "$models/$f" | cut -d' ' -f1)  $f"; done
} > "$here/router.sha256"
echo "wrote scripts/laya/router.sha256; commit it, then: git tag $tag && git push origin $tag"
