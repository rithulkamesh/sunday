#!/usr/bin/env bash
# Install Sunday on macOS (Apple silicon) from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/rithulkamesh/sunday/main/scripts/install.sh | bash
#
# Downloads the app, checks its checksum, clears Gatekeeper quarantine, ad-hoc codesigns it (no Apple Developer
# account needed) and installs it to ~/Applications. Then it fetches the speech models and, if one is published, the
# optional routing model into ~/.cache/sunday. Set SUNDAY_SKIP_MODELS=1 to skip that part.
# You still need a model provider (Ollama, LM Studio or MLX): see docs/PROVIDERS.md.
set -euo pipefail

REPO="rithulkamesh/sunday"
ASSET="Sunday-macOS-arm64.zip"
APP_DST="$HOME/Applications"

if [[ -t 1 ]]; then B=$'\033[1m' D=$'\033[2m' G=$'\033[32m' C=$'\033[36m' R=$'\033[31m' Z=$'\033[0m'; else B= D= G= C= R= Z=; fi
step() { printf '\n  %s▸%s %s%s%s\n' "$C" "$Z" "$B" "$*" "$Z"; }
ok()   { printf '  %s✓%s %s\n' "$G" "$Z" "$*"; }
fail() { printf '  %s✗%s %s\n' "$R" "$Z" "$*" >&2; exit 1; }

[[ "$(uname -sm)" == "Darwin arm64" ]] || fail "Sunday needs a Mac with Apple silicon"
printf '\n  %sSunday%s  %smacOS installer%s\n' "$B" "$Z" "$D" "$Z"

step "checking latest release"
TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
[[ -n "$TAG" ]] || fail "could not find a release; check your network"
ok "$TAG"

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
BASE="https://github.com/$REPO/releases/download/$TAG"

step "downloading $ASSET"
curl -fL --progress-bar -o "$TMP/$ASSET" "$BASE/$ASSET"
curl -fsSL -o "$TMP/$ASSET.sha256" "$BASE/$ASSET.sha256"
(cd "$TMP" && shasum -a 256 -c "$ASSET.sha256" >/dev/null) || fail "checksum mismatch"
ok "$(du -h "$TMP/$ASSET" | awk '{print $1}') verified"

step "trusting locally (quarantine + ad-hoc sign)"
ditto -x -k "$TMP/$ASSET" "$TMP/app"
APP="$TMP/app/Sunday.app"
[[ -d "$APP" ]] || fail "zip layout unexpected: no Sunday.app"
xattr -cr "$APP"
codesign --force --deep -s - "$APP" >/dev/null
ok "ad-hoc signed, quarantine cleared"

step "installing"
mkdir -p "$APP_DST"
rm -rf "$APP_DST/Sunday.app"
cp -R "$APP" "$APP_DST/"
ok "$APP_DST/Sunday.app"

if [[ -z "${SUNDAY_SKIP_MODELS:-}" ]]; then
  # The model scripts at the same tag, laid out as in the repo (fetch-router.sh reads laya/router.sha256).
  step "fetching models"
  mkdir -p "$TMP/scripts/laya"
  raw="https://raw.githubusercontent.com/$REPO/$TAG/scripts"
  for f in fetch-models.sh fetch-router.sh laya/router.sha256; do curl -fsSL -o "$TMP/scripts/$f" "$raw/$f" || true; done
  chmod +x "$TMP"/scripts/*.sh
  bash "$TMP/scripts/fetch-models.sh"
  bash "$TMP/scripts/fetch-router.sh" || printf '  %srouting model skipped; Sunday works without it%s\n' "$D" "$Z"
fi

printf '\n  %s%sinstalled %s%s\n  %sstart Ollama or LM Studio (docs/PROVIDERS.md), then open Sunday from ~/Applications%s\n\n' "$G" "$B" "$TAG" "$Z" "$D" "$Z"
