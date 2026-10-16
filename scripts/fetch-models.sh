#!/usr/bin/env bash
# One-time: download VAD, Parakeet (STT) and Kokoro (TTS) into ~/.cache/sunday/models.
# The app never downloads anything itself.
set -euo pipefail
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}"
rel=https://github.com/k2-fsa/sherpa-onnx/releases/download
mkdir -p "$dir" && cd "$dir"

[ -f silero_vad.onnx ] || curl -fSL -O $rel/asr-models/silero_vad.onnx
for m in "asr-models sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8" "tts-models kokoro-en-v0_19"; do
  set -- $m
  [ -d "$2" ] && continue
  curl -fSL -O "$rel/$1/$2.tar.bz2" && tar xf "$2.tar.bz2" && rm "$2.tar.bz2"
done
echo "models in $dir"
