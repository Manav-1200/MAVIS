#!/usr/bin/env bash
set -euo pipefail

VOICE_DIR="$HOME/.local/share/piper-voices"
mkdir -p "$VOICE_DIR"
cd "$VOICE_DIR"

BASE_URL="https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/medium"

echo "Downloading Piper voice: en_US-lessac-medium..."

wget -q --show-progress "${BASE_URL}/en_US-lessac-medium.onnx" -O en_US-lessac-medium.onnx || {
    echo "ERROR: Failed to download ONNX model"
    exit 1
}

wget -q --show-progress "${BASE_URL}/en_US-lessac-medium.onnx.json" -O en_US-lessac-medium.onnx.json || {
    echo "ERROR: Failed to download voice config"
    exit 1
}

echo ""
echo "Voice installed successfully to $VOICE_DIR"
ls -la "$VOICE_DIR"