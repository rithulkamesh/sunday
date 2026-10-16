#!/usr/bin/env bash
# One-time: download VAD, wake-word spotter, speaker-ID, Parakeet (STT) and Kokoro (TTS) into ~/.cache/sunday/models.
# The app never downloads anything itself.
set -euo pipefail
dir="${SUNDAY_MODELS:-$HOME/.cache/sunday/models}"
rel=https://github.com/k2-fsa/sherpa-onnx/releases/download
mkdir -p "$dir" && cd "$dir"

[ -f silero_vad.onnx ] || curl -fSL -O $rel/asr-models/silero_vad.onnx
# Speaker verification ("learn my voice"); the release tag really is spelled "recongition".
[ -f wespeaker_en_voxceleb_resnet34_LM.onnx ] || curl -fSL -O $rel/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx
for m in "kws-models sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01" "asr-models sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8" "tts-models kokoro-en-v0_19"; do
  set -- $m
  [ -d "$2" ] && continue
  curl -fSL -O "$rel/$1/$2.tar.bz2" && tar xf "$2.tar.bz2" && rm "$2.tar.bz2"
done
echo "models in $dir"
