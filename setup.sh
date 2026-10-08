#!/usr/bin/env bash
# setup.sh
# First-time setup: how MAVIS should listen.
set -euo pipefail

# Works from any directory: paths are relative to this script.
ROOT="$(cd "$(dirname "$0")" && pwd)"
CONFIG="$ROOT/config/config.toml"
TOGGLE="$ROOT/toggle_listen.sh"

[ -f "$CONFIG" ] || { echo "config/config.toml not found next to setup.sh."; exit 1; }

echo "How should MAVIS listen?"
echo "  1) Always — it hears you whenever you speak (default)"
echo "  2) Push to talk — it listens only after a hotkey or a tap on the orb"
read -rp "Choose 1 or 2 [1]: " choice

case "${choice:-1}" in
    2) mode="push" ;;
    *) mode="always" ;;
esac

# Replace listen_mode under [voice], or add it there.
python3 - "$CONFIG" "$mode" <<'PY'
import re, sys
path, mode = sys.argv[1], sys.argv[2]
text = open(path).read()
line = f'listen_mode = "{mode}"'
if re.search(r'(?m)^listen_mode\s*=.*$', text):
    text = re.sub(r'(?m)^listen_mode\s*=.*$', line, text, count=1)
else:
    text = re.sub(r'(?m)^\[voice\]\s*$', '[voice]\n' + line, text, count=1)
open(path, "w").write(text)
PY
echo "Saved: listen_mode = \"$mode\" in $CONFIG"

[ "$mode" = "push" ] || exit 0

chmod +x "$TOGGLE"
echo
echo "Bind a key to: $TOGGLE"
case "${XDG_CURRENT_DESKTOP:-}${NIRI_SOCKET:+niri}${SWAYSOCK:+sway}${HYPRLAND_INSTANCE_SIGNATURE:+hyprland}" in
    *niri*)     echo "niri (config.kdl, in binds):  Mod+Space { spawn \"$TOGGLE\"; }" ;;
    *sway*)     echo "sway (config):  bindsym \$mod+space exec $TOGGLE" ;;
    *yprland*)  echo "Hyprland (hyprland.conf):  bind = SUPER, SPACE, exec, $TOGGLE" ;;
    *)          echo "Use your desktop's keyboard settings to add a custom shortcut that runs it." ;;
esac
echo "Or tap the orb. Press once, then speak; MAVIS hears one sentence and stops."