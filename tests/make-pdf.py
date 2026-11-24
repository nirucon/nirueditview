#!/usr/bin/env python3
"""A two-page test form: page 1 A4 upright, page 2 A4 with /Rotate 90.
Usage: make-pdf.py OUT.pdf"""
import sys

def stream(body):
    return b"<< /Length %d >>\nstream\n" % len(body) + body + b"\nendstream"

objs = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F1 7 0 R >> >> >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 5 0 R >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Rotate 90 /Contents 6 0 R >>",
    stream(b"BT /F1 24 Tf 72 770 Td (Test form) Tj ET 0.8 0.8 0.8 rg 72 600 300 40 re f"),
    stream(b"BT /F1 24 Tf 72 770 Td (Page two) Tj ET"),
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
]
out = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
offsets = []
for i, o in enumerate(objs, 1):
    offsets.append(len(out))
    out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
for off in offsets:
    out += b"%010d 00000 n \n" % off
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
open(sys.argv[1], "wb").write(out)
