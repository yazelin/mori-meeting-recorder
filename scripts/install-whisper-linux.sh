#!/usr/bin/env bash
set -euo pipefail
ver="${WHISPER_VERSION:-v1.7.0}"
mkdir -p ~/.mori/bin ~/.mori/models
if [ ! -x ~/.mori/bin/whisper-cli ]; then
  echo "→ downloading whisper.cpp ${ver}…"
  url="https://github.com/ggerganov/whisper.cpp/releases/download/${ver}/whisper-bin-x64.zip"
  curl -L -o /tmp/whisper.zip "$url"
  unzip -o /tmp/whisper.zip -d /tmp/whisper-unzip
  cp /tmp/whisper-unzip/main ~/.mori/bin/whisper-cli 2>/dev/null || \
    cp /tmp/whisper-unzip/whisper-cli ~/.mori/bin/whisper-cli
  chmod +x ~/.mori/bin/whisper-cli
  rm -rf /tmp/whisper-unzip /tmp/whisper.zip
fi
if [ ! -f ~/.mori/models/ggml-small.bin ]; then
  echo "→ downloading ggml-small model…"
  curl -L -o ~/.mori/models/ggml-small.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
fi
echo "✓ ready: ~/.mori/bin/whisper-cli + ~/.mori/models/ggml-small.bin"
