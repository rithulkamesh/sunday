#!/usr/bin/env bash
# Optional: download the routing model (Hugging Face) and its sidecar program (GitHub Release) into ~/.cache/sunday.
# The app never downloads anything itself. Weights are checked against scripts/laya/router.sha256, which lives in
# git, so a changed Hugging Face repo can't slip in. The program is ad-hoc signed here, on your machine.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
models="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}"
bin="$models/../bin"
repo="${SUNDAY_ROUTER_REPO:-rithulkamesh/sunday-laya-router}"
sums="$here/laya/router.sha256"
gh_repo=rithulkamesh/sunday

[ "$(uname -sm)" = "Darwin arm64" ] || { echo "the prebuilt router is for Apple silicon; build it with scripts/build-laya.sh" >&2; exit 1; }
[ -f "$sums" ] || { echo "no router has been published yet (scripts/laya/router.sha256 is missing)" >&2; exit 1; }

# The revision is the first line of the checksum file's comment header: "# rev <commit>".
rev="$(sed -n 's/^# rev //p' "$sums")"
dest="$models/laya-routing"
mkdir -p "$dest/tokenizer" "$bin"

while read -r sum file; do
  [[ "$sum" == \#* || -z "$sum" ]] && continue
  if [ -f "$dest/$file" ] && [ "$(shasum -a 256 "$dest/$file" | cut -d' ' -f1)" = "$sum" ]; then continue; fi
  echo "downloading $file"
  curl -fSL --retry 3 -C - -o "$dest/$file" "https://huggingface.co/$repo/resolve/$rev/$file"
  [ "$(shasum -a 256 "$dest/$file" | cut -d' ' -f1)" = "$sum" ] || { echo "checksum mismatch for $file" >&2; rm -f "$dest/$file"; exit 1; }
done < "$sums"

# The program: built by CI on a router-v* tag, with its checksum beside it.
tag="$(sed -n 's/^# release //p' "$sums")"
asset="laya-router-aarch64-apple-darwin"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
curl -fSL --retry 3 -o "$tmp/$asset" "https://github.com/$gh_repo/releases/download/$tag/$asset"
curl -fSL --retry 3 -o "$tmp/$asset.sha256" "https://github.com/$gh_repo/releases/download/$tag/$asset.sha256"
(cd "$tmp" && shasum -a 256 -c "$asset.sha256")
install -m 755 "$tmp/$asset" "$bin/laya-router"
xattr -d com.apple.quarantine "$bin/laya-router" 2>/dev/null || true
codesign --force --sign - "$bin/laya-router"   # ad-hoc: arm64 refuses unsigned code, and this needs no developer account
echo "router installed: $dest and $bin/laya-router"
