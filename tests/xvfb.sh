#!/bin/sh
# End to end in a private Xvfb: pictures shown, turned and saved as a copy,
# the gallery and the slideshow, and on a PDF a signature, text and the
# date put where they were clicked, on a plain and on a rotated page, and
# saved as a signed copy that keeps the original bytes.
# Needs Xvfb, xdotool, ffmpeg (ffprobe), poppler (pdftoppm, pdftotext) and
# python3. Usage: tests/xvfb.sh PATH/TO/nirueditview [SHOTS-DIR]
set -eu
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
HERE=$(cd "$(dirname "$0")" && pwd)
SHOTS=${2:-}
SB=$(mktemp -d)
PIDS=""
cleanup() {
  for p in $PIDS; do kill "$p" 2>/dev/null || true; done
  if [ -n "${KEEP:-}" ]; then echo "sandbox $SB"; else rm -rf "$SB"; fi
}
trap cleanup EXIT INT TERM
export HOME=$SB/home XDG_CONFIG_HOME=$SB/home/.config XDG_CACHE_HOME=$SB/home/.cache XDG_DATA_HOME=$SB/home/.local/share
mkdir -p "$HOME/Pictures" "$HOME/Documents"
P=$HOME/Pictures D=$HOME/Documents
ffmpeg -loglevel error -f lavfi -i testsrc=size=1200x800 -frames:v 1 "$P/a1.jpg"
ffmpeg -loglevel error -f lavfi -i testsrc2=size=640x480 -frames:v 1 "$P/a2.png"
ffmpeg -loglevel error -f lavfi -i "color=c=blue:size=300x200" -frames:v 1 "$P/a10.jpg"
python3 "$HERE/make-pdf.py" "$D/form.pdf"
cp "$D/form.pdf" "$SB/original.pdf"

D_NUM=$((60 + $$ % 20))
Xvfb ":$D_NUM" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
PIDS="$PIDS $!"
export DISPLAY=:$D_NUM
sleep 1
fail() { echo "FAIL: $*" >&2; [ -f "$SB/out.log" ] && tail -20 "$SB/out.log" >&2; exit 1; }
shot() { [ -n "$SHOTS" ] && ffmpeg -loglevel error -y -f x11grab -video_size 1280x900 -i "$DISPLAY" -frames:v 1 "$SHOTS/$1.png" || true; }
grab() { ffmpeg -loglevel error -y -f x11grab -video_size 1280x900 -i "$DISPLAY" -frames:v 1 -f image2 -c:v ppm "$1"; }
wait_for() { n=$(( $1 * 10 )); shift; while [ $n -gt 0 ]; do "$@" && return 0; sleep 0.1; n=$((n - 1)); done; return 1; }
dims() { ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=s=x:p=0 "$1"; }
start() {
  "$BIN" "$@" >"$SB/out.log" 2>&1 &
  APP=$!
  PIDS="$PIDS $APP"
  W=$(xdotool search --sync --onlyvisible --class nirueditview | head -1)
  xdotool windowmove "$W" 0 0 windowsize "$W" 1200 860
  xdotool windowfocus "$W" 2>/dev/null || true
  sleep 0.8
}
key() { xdotool key --window "$W" "$@"; sleep 0.25; }
stop() { kill "$APP" 2>/dev/null || true; wait "$APP" 2>/dev/null || true; }

# --- pictures ---------------------------------------------------------------
start "$P/a1.jpg"
shot picture
rss=$(awk '/VmRSS/ {print $2}' /proc/$APP/status)
[ "$rss" -lt 120000 ] || fail "memory $rss kB"
echo "picture shown, ${rss} kB"
key Right; sleep 0.3
xdotool getwindowname "$W" | grep -q "a2.png" || fail "Right did not go to a2.png (natural order)"
key Right
xdotool getwindowname "$W" | grep -q "a10.jpg" || fail "Right did not go to a10.jpg"
key Home
key g; sleep 1.5; shot gallery
[ -n "$(ls "$XDG_CACHE_HOME/thumbnails/large/" 2>/dev/null)" ] || fail "no shared thumbnails"
key Escape
key r; key ctrl+s
wait_for 5 test -f "$P/a1-edited.jpg" || fail "Ctrl+S made no copy"
[ "$(dims "$P/a1-edited.jpg")" = "800x1200" ] || fail "the copy is not turned: $(dims "$P/a1-edited.jpg")"
[ "$(dims "$P/a1.jpg")" = "1200x800" ] || fail "the original changed"
echo "rotate and save a copy ok"
key s; sleep 0.5; shot slideshow; key Escape
key i; sleep 0.3; shot info; key i
stop

# --- PDF: signature, text, date ----------------------------------------------
start "$D/form.pdf"
sleep 1.5; shot pdf
grab "$SB/screen1.ppm"
# Sign: a new signature drawn in the pad, then put on page 1.
key shift+s; sleep 0.3
eval "$(python3 "$HERE/geometry.py" pad "$SB/screen1.ppm")"
xdotool mousemove "$PX" "$PY" mousedown 1
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
  xdotool mousemove $((PX + i * 18)) $((PY + (i % 3) * 14 - 10)); sleep 0.02
done
xdotool mouseup 1; sleep 0.2; shot pad
key Return
[ -f "$XDG_DATA_HOME/nirueditview/signature.png" ] || fail "the signature was not kept"
eval "$(python3 "$HERE/geometry.py" page1 "$SB/screen1.ppm")"
xdotool mousemove "$CX" "$CY"; sleep 0.3; xdotool click 1; sleep 0.3
# Text, placed lower on the same page.
key t; xdotool type --window "$W" "Hello NIRU" 2>/dev/null; key Return
xdotool mousemove "$CX" $((CY + 120)); sleep 0.3; xdotool click 1; sleep 0.3
# The date on page 2 (rotated 90 in the file).
key Next; sleep 1.5
grab "$SB/screen2.ppm"
eval "$(python3 "$HERE/geometry.py" page2 "$SB/screen2.ppm")"
key d; xdotool mousemove "$C2X" "$C2Y"; sleep 0.3; xdotool click 1; sleep 0.3
shot stamped
key ctrl+s
wait_for 8 test -f "$D/form-signed.pdf" || fail "no signed copy"
cmp -s "$D/form.pdf" "$SB/original.pdf" || fail "the original PDF changed"
python3 - "$SB/original.pdf" "$D/form-signed.pdf" <<'EOF' || fail "the original bytes are not kept in the copy"
import sys
a = open(sys.argv[1], 'rb').read(); b = open(sys.argv[2], 'rb').read()
sys.exit(0 if b.startswith(a) and len(b) > len(a) else 1)
EOF
pdftotext "$D/form-signed.pdf" - | grep -q "Hello NIRU" || fail "the text is not in the PDF"
pdftotext "$D/form-signed.pdf" - | grep -q "$(date +%Y-%m-%d)" || fail "the date is not in the PDF"
# Where it landed: the same place on the page as on the screen.
python3 "$HERE/geometry.py" check "$SB/original.pdf" "$D/form-signed.pdf" "$SB" \
  "$REL1X" "$REL1Y" "$REL2X" "$REL2Y" || fail "a stamp is not where it was clicked"
echo "signature, text and date ok, also on a rotated page"
stop
echo "PASS"
