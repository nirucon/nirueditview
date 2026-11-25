#!/usr/bin/env python3
"""A one-page A4 form (AcroForm): a text field "name", a check box "agree"
and a drop-down "colour" (Red, Green, Blue). Usage: make-form.py OUT.pdf"""
import sys

def stream(dict_body, body):
    return b"<< %s /Length %d >>\nstream\n" % (dict_body, len(body)) + body + b"\nendstream"

objs = [
    b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R 6 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 9 0 R >> >> >> >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 10 0 R"
    b" /Resources << /Font << /F1 9 0 R >> >> /Annots [4 0 R 5 0 R 6 0 R] >>",
    b"<< /FT /Tx /T (name) /Type /Annot /Subtype /Widget /Rect [150 700 400 722] /P 3 0 R"
    b" /DA (/Helv 12 Tf 0 g) /MK << /BG [0.9 0.95 1] >> /F 4 >>",
    b"<< /FT /Btn /T (agree) /Type /Annot /Subtype /Widget /Rect [150 650 166 666] /P 3 0 R"
    b" /V /Off /AS /Off /F 4 /AP << /N << /Yes 7 0 R /Off 8 0 R >> >> >>",
    b"<< /FT /Ch /Ff 131072 /T (colour) /Type /Annot /Subtype /Widget /Rect [150 600 300 622] /P 3 0 R"
    b" /Opt [(Red) (Green) (Blue)] /DA (/Helv 12 Tf 0 g) /F 4 >>",
    stream(b"/Type /XObject /Subtype /Form /BBox [0 0 16 16]", b"0 g 3 3 10 10 re f"),
    stream(b"/Type /XObject /Subtype /Form /BBox [0 0 16 16]", b""),
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    stream(b"", b"BT /F1 20 Tf 72 770 Td (Application) Tj ET"
           b" BT /F1 12 Tf 72 706 Td (Name) Tj 0 -50 Td (I agree) Tj 0 -50 Td (Colour) Tj ET"),
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
