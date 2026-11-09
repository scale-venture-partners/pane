#!/bin/sh
# Regenerates docs/screenshots/*.png from `pane --demo` (fictional data, no network).
# Needs: cargo, tmux, python3, and Google Chrome (override with CHROME=/path/to/chrome).
set -eu

cd "$(dirname "$0")/.."
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
OUT=docs/screenshots
TMP="$(mktemp -d)"
trap 'tmux kill-session -t pane-shots 2>/dev/null || true; rm -rf "$TMP"' EXIT

cargo build --quiet
BIN="$PWD/target/debug/pane"
mkdir -p "$OUT"

# shot NAME KEYS...: start pane, send each key (tmux syntax), capture, render to PNG.
shot() {
  name="$1"; shift
  tmux kill-session -t pane-shots 2>/dev/null || true
  tmux new-session -d -s pane-shots -x 150 -y 40 "env COLORTERM=truecolor $BIN --demo"
  sleep 1
  for key in "$@"; do
    tmux send-keys -t pane-shots "$key"
    sleep 0.4
  done
  sleep 0.5
  tmux capture-pane -t pane-shots -e -p > "$TMP/$name.ansi"
  python3 scripts/ansi2html.py pane < "$TMP/$name.ansi" > "$TMP/$name.html"
  "$CHROME" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
    --window-size=1360,780 --screenshot="$PWD/$OUT/$name.png" "file://$TMP/$name.html" >/dev/null 2>&1
  echo "wrote $OUT/$name.png"
}

shot overview Enter
shot description Enter PageDown PageDown
shot diff Enter d "]" "]"
shot repair j j j j j j Enter
shot issues Tab Enter
