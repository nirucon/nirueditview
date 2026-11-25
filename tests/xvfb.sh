#!/bin/sh
# End to end in a private Xvfb: pictures shown, turned and saved as a copy,
# the gallery and the slideshow, and on a PDF a signature, text and the
# date put where they were clicked, on a plain and on a rotated page, and
# saved as a signed copy that keeps the original bytes. Then: light and
# straighten, save as another format, a copy without location, the
# wallpaper; on PDFs find, select and copy text, the pages view (turn,
# remove, add a PDF, save, save a range) and filling in a form.
# Needs Xvfb, xdotool, xclip, ffmpeg (ffprobe), poppler (pdftoppm, pdfinfo,
# pdftotext) and python3. Usage: tests/xvfb.sh PATH/TO/nirueditview [SHOTS-DIR]
# Everything runs with HOME and every XDG folder in a temporary sandbox.
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
export HOME=$SB/home XDG_CONFIG_HOME=$SB/home/.config XDG_CACHE_HOME=$SB/home/.cache \
  XDG_DATA_HOME=$SB/home/.local/share XDG_STATE_HOME=$SB/home/.local/state XDG_RUNTIME_DIR=$SB/run
mkdir -p "$HOME/Pictures" "$HOME/Documents" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
# NIRUWALLPAPER stand-in: it writes down what it was asked.
printf '#!/bin/sh\necho "$@" >> "%s/wallpaper.log"\n' "$SB" > "$SB/niruwallpaper"
chmod +x "$SB/niruwallpaper"
export NIRUEDITVIEW_NIRUWALLPAPER=$SB/niruwallpaper
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
  xdotool windowfocus --sync "$W" 2>/dev/null || true
  sleep 0.8
}
key() { xdotool windowfocus --sync "$W"; xdotool key "$@"; sleep 0.25; }
type_text() { xdotool windowfocus --sync "$W"; xdotool type --delay 40 "$1"; sleep 0.3; }
gone() { ! xdotool search --onlyvisible --class nirueditview >/dev/null 2>&1; }
# The window is gone before the next one opens, so keys go to the new one.
stop() { kill "$APP" 2>/dev/null || true; wait "$APP" 2>/dev/null || true; wait_for 5 gone || true; }

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
key t; type_text "Hello NIRU"; key Return
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

# --- pictures: light, straighten, save as, private copy, wallpaper -----------
mean() {
  ffmpeg -loglevel error -y -i "$1" -f image2 -c:v ppm "$SB/m.ppm"
  python3 - "$SB/m.ppm" <<'EOF'
import sys
d = open(sys.argv[1], 'rb').read()
parts, i = [], 0
while len(parts) < 4:
    while d[i:i + 1].isspace():
        i += 1
    j = i
    while not d[j:j + 1].isspace():
        j += 1
    parts.append(d[i:j]); i = j
px = d[i + 1:]
print(sum(px) // len(px))
EOF
}
start "$P/a2.png"
before=$(mean "$P/a2.png")
key l; key Up; key Up; key Up; key Up; shot light; key Return
key e; key BackSpace BackSpace BackSpace; type_text "jpg 80"; key Return
wait_for 5 test -f "$P/a2.jpg" || fail "E (save as jpg 80) made no a2.jpg"
after=$(mean "$P/a2.jpg")
[ "$after" -gt $((before + 10)) ] || fail "not brighter after L: $before -> $after"
key z; key Right; key Right; key Right; key Right; sleep 0.3; shot straighten; key Return
key ctrl+s
wait_for 5 test -f "$P/a2-edited.jpg" || fail "no copy after straightening"
d=$(dims "$P/a2-edited.jpg"); w=${d%x*}; h=${d#*x}
{ [ "$w" -lt 640 ] && [ "$w" -gt 560 ] && [ "$h" -lt 480 ]; } || fail "the straightened copy is $d, not cut a little smaller"
echo "light ($before -> $after), save as jpg 80, straighten ($d) ok"
stop

python3 "$HERE/add-gps.py" "$P/a10.jpg" "$P/phone.jpg"
start "$P/phone.jpg"
key i; sleep 0.3; shot location; key i
key ctrl+l
wait_for 5 test -f "$P/phone-private.jpg" || fail "Ctrl+L made no private copy"
python3 - "$P/phone.jpg" "$P/phone-private.jpg" <<'EOF' || fail "the private copy still has EXIF or another picture"
import sys
a = open(sys.argv[1], 'rb').read(); b = open(sys.argv[2], 'rb').read()
assert b"Exif" in a and b"PhoneCo" in a
assert b"Exif" not in b and b"PhoneCo" not in b, "metadata left"
sos = lambda d: d.index(b"\xff\xda")
assert a[sos(a):] == b[sos(b):], "the picture changed"
EOF
[ "$(dims "$P/phone-private.jpg")" = "300x200" ] || fail "the private copy has other pixels"
echo "private copy without location ok"
key W
wait_for 5 test -f "$SB/wallpaper.log" || fail "Shift+W did not call niruwallpaper"
grep -qx "set $P/phone-private.jpg" "$SB/wallpaper.log" || fail "wallpaper: $(cat "$SB/wallpaper.log")"
echo "wallpaper via niruwallpaper set ok"
stop

# --- PDF: find, select and copy ----------------------------------------------
cp "$SB/original.pdf" "$D/text.pdf"
start "$D/text.pdf"
sleep 1.5
key ctrl+f; type_text "page two"; sleep 1.5; shot find
grab "$SB/find.ppm"
python3 "$HERE/geometry.py" found "$SB/find.ppm" || fail "Ctrl+F did not light up the text"
key Escape; key Home; sleep 1.2
grab "$SB/top.ppm"
eval "$(python3 "$HERE/geometry.py" title "$SB/top.ppm")"
xdotool mousemove "$TX0" "$TY"; sleep 0.2
xdotool keydown shift mousedown 1
xdotool mousemove $(( (TX0 + TX1) / 2 )) "$TY"; sleep 0.1
xdotool mousemove "$TX1" "$TY"; sleep 0.2
xdotool mouseup 1 keyup shift; sleep 0.3; shot selected
key ctrl+c; sleep 0.5
got=$(xclip -o -selection clipboard 2>/dev/null || true)
[ "$got" = "Test form" ] || fail "Ctrl+C copied '$got', not 'Test form'"
echo "find and copy text ok"
stop

# --- PDF: pages --------------------------------------------------------------
start "$D/text.pdf"
sleep 1
key p; sleep 1.5; shot pages
key r; key Right; key Delete
key m; type_text "text.pdf"; key Return; sleep 0.5; shot pages-merged
key ctrl+s
wait_for 8 test -f "$D/text-pages.pdf" || fail "Ctrl+S in the pages view made no copy"
cmp -s "$D/text.pdf" "$SB/original.pdf" || fail "the original PDF changed"
pdfinfo "$D/text-pages.pdf" | grep -q "^Pages: *3$" || fail "not 3 pages: $(pdfinfo "$D/text-pages.pdf" | grep Pages)"
rots=$(pdfinfo -f 1 -l 3 "$D/text-pages.pdf" | awk '/ rot:/ {printf "%s ", $4}')
[ "$rots" = "90 0 90 " ] || fail "the pages are turned '$rots'"
pdftotext "$D/text-pages.pdf" - | tr '\n' ' ' | grep -q "Test form.*Test form.*Page two" || fail "the pages are not in the new order"
xdotool getwindowname "$W" | grep -q "text-pages.pdf" || fail "the new file is not shown"
key p; sleep 1; key e; key BackSpace; type_text "2-3"; key Return
wait_for 5 test -f "$D/text-pages-pages-2-3.pdf" || fail "E made no file of pages 2-3"
pdfinfo "$D/text-pages-pages-2-3.pdf" | grep -q "^Pages: *2$" || fail "the range is not 2 pages"
echo "pages: turn, remove, add a PDF, save, save a range ok"
stop

# --- PDF: a form -------------------------------------------------------------
python3 "$HERE/make-form.py" "$D/apply.pdf"
cp "$D/apply.pdf" "$SB/apply.pdf"
start "$D/apply.pdf"
sleep 1
key e; key Tab; key Return; type_text "Ada"; key Return
key Tab; key space
key Tab; key Return; type_text "2"; key Return
sleep 0.3; shot form
key ctrl+s
wait_for 8 test -f "$D/apply-filled.pdf" || fail "no filled copy"
cmp -s "$D/apply.pdf" "$SB/apply.pdf" || fail "the original form changed"
python3 - "$SB/apply.pdf" "$D/apply-filled.pdf" <<'EOF' || fail "the values are not in the filled copy"
import re, sys
a = open(sys.argv[1], 'rb').read(); b = open(sys.argv[2], 'rb').read()
assert b.startswith(a), "not an incremental update"
new = b[len(a):]
assert re.search(rb"/V\s*\(Ada\)", new), "name"
assert re.search(rb"/V\s*/Yes", new) and re.search(rb"/AS\s*/Yes", new), "check box"
assert re.search(rb"/V\s*\(Green\)", new), "colour"
EOF
# Poppler draws the values where the fields are.
pdftoppm -r 40 -singlefile "$SB/apply.pdf" "$SB/f0"
pdftoppm -r 40 -singlefile "$D/apply-filled.pdf" "$SB/f1"
cmp -s "$SB/f0.ppm" "$SB/f1.ppm" && fail "the filled form looks the same"
echo "form filled in: text, check box, choice ok"
stop
echo "PASS"
