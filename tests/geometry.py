#!/usr/bin/env python3
"""Screen geometry for tests/xvfb.sh: where the pad, the pages and the
title are in a screenshot, whether found text is lit up, and whether the
stamps in the signed PDF are where they were clicked (relative to their
page)."""
import os, subprocess, sys

WIN_W, WIN_H = 1200, 860
# The test form: page 1 is 595 x 842 points shown, page 2 842 x 595.
ASPECT = {1: 842 / 595, 2: 595 / 842}

def ppm(path):
    data = open(path, "rb").read()
    parts, i = [], 0
    while len(parts) < 4:
        while data[i:i + 1].isspace():
            i += 1
        j = i
        while not data[j:j + 1].isspace():
            j += 1
        parts.append(data[i:j]); i = j
    w, h = int(parts[1]), int(parts[2])
    return w, h, data[i + 1:]

def white(px, w, x, y):
    k = (y * w + x) * 3
    return px[k] > 250 and px[k + 1] > 250 and px[k + 2] > 250

def page_box(path, n):
    w, h, px = ppm(path)
    cx = WIN_W // 2
    # The first run of 40 white rows: a page, not the edge of the one above.
    top = next(y for y in range(h - 40) if all(white(px, w, cx, y + k) for k in range(40)))
    row = top + 6
    left = cx
    while left > 0 and white(px, w, left - 1, row):
        left -= 1
    right = cx
    while right < WIN_W - 1 and white(px, w, right + 1, row):
        right += 1
    pw = right - left + 1
    return left, top, pw, pw * ASPECT[n]

def render(pdf, page, out):
    subprocess.run(["pdftoppm", "-r", "40", "-f", str(page), "-l", str(page), "-singlefile", pdf, out], check=True)
    return ppm(out + ".ppm")

def check(orig, signed, tmp, expected):
    ok = True
    for page, (ex, ey) in expected.items():
        w, h, a = render(orig, page, os.path.join(tmp, "o%d" % page))
        _, _, b = render(signed, page, os.path.join(tmp, "s%d" % page))
        xs, ys = [], []
        for y in range(h):
            for x in range(w):
                k = (y * w + x) * 3
                if abs(a[k] - b[k]) + abs(a[k + 1] - b[k + 1]) + abs(a[k + 2] - b[k + 2]) > 60:
                    rx, ry = x / w, y / h
                    if abs(rx - ex) < 0.2 and abs(ry - ey) < 0.07:
                        xs.append(rx); ys.append(ry)
        if not xs:
            print("page %d: nothing new near %.2f,%.2f" % (page, ex, ey), file=sys.stderr)
            ok = False
            continue
        cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
        print("page %d: expected %.3f,%.3f found %.3f,%.3f" % (page, ex, ey, cx, cy), file=sys.stderr)
        if abs(cx - ex) > 0.03 or abs(cy - ey) > 0.03:
            ok = False
    return ok

cmd = sys.argv[1]
if cmd == "pad":
    print("PX=%d PY=%d" % (WIN_W // 2 - 110, WIN_H // 2))
elif cmd == "page1":
    l, t, w, h = page_box(sys.argv[2], 1)
    rx, ry = 0.30, 0.25
    print("CX=%d CY=%d REL1X=%.4f REL1Y=%.4f" % (l + rx * w, t + ry * h, rx, ry))
elif cmd == "page2":
    l, t, w, h = page_box(sys.argv[2], 2)
    rx, ry = 0.60, 0.40
    print("C2X=%d C2Y=%d REL2X=%.4f REL2Y=%.4f" % (l + rx * w, t + ry * h, rx, ry))
elif cmd == "found":
    # Text found by Ctrl+F is lit up in orange (the current place).
    w, h, px = ppm(sys.argv[2])
    n = 0
    for k in range(0, len(px) - 2, 3):
        r, g, b = px[k], px[k + 1], px[k + 2]
        if r > 235 and 150 < g < 220 and b < 170:
            n += 1
    print("orange pixels: %d" % n, file=sys.stderr)
    sys.exit(0 if n > 30 else 1)
elif cmd == "title":
    # "Test form" on page 1: 72..173 across, 55..77 down, of 595 x 842.
    l, t, w, h = page_box(sys.argv[2], 1)
    print("TX0=%d TX1=%d TY=%d" % (l + 0.115 * w, l + 0.30 * w, t + 0.078 * h))
elif cmd == "check":
    orig, signed, tmp = sys.argv[2:5]
    r1 = (float(sys.argv[5]), float(sys.argv[6]))
    r2 = (float(sys.argv[7]), float(sys.argv[8]))
    sys.exit(0 if check(orig, signed, tmp, {1: r1, 2: r2}) else 1)
