#!/usr/bin/env bash
# toggle_listen.sh
# Push to talk: start (or stop) MAVIS listening for one utterance.
# Bind this to a key in your compositor; setup.sh shows how.
set -euo pipefail
exec python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
try:
    s.connect("/tmp/mavis_hotkey.sock")
except OSError:
    sys.exit("MAVIS is not running")
s.sendall(b"toggle_listen\n")
'