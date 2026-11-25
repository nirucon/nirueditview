#!/usr/bin/env python3
"""Give a JPEG the EXIF a phone writes: camera make and model and a GPS
position (58.2 N, 11.9 E). Usage: add-gps.py IN.jpg OUT.jpg"""
import struct, sys

def ifd(entries, start, extra_at):
    """entries: (tag, type, count, payload bytes). Returns IFD bytes and the
    data that did not fit in the entries, placed at extra_at."""
    body = struct.pack("<H", len(entries))
    extra = b""
    for tag, typ, count, payload in entries:
        if len(payload) <= 4:
            body += struct.pack("<HHI", tag, typ, count) + payload.ljust(4, b"\0")
        else:
            body += struct.pack("<HHII", tag, typ, count, extra_at + len(extra))
            extra += payload + (b"\0" if len(payload) % 2 else b"")
    body += struct.pack("<I", 0)
    return body, extra

def rationals(*vals):
    return b"".join(struct.pack("<II", n, d) for n, d in vals)

make, model = b"PhoneCo\0", b"Phone 9\0"
ifd0_at = 8
n0 = 3
ifd0_len = 2 + n0 * 12 + 4
# IFD0: Make, Model, GPS pointer (filled once the GPS IFD's place is known).
extra0_at = ifd0_at + ifd0_len
extra0 = make + model
gps_at = extra0_at + len(extra0)
ifd0 = struct.pack("<H", n0)
ifd0 += struct.pack("<HHII", 0x010F, 2, len(make), extra0_at)
ifd0 += struct.pack("<HHII", 0x0110, 2, len(model), extra0_at + len(make))
ifd0 += struct.pack("<HHII", 0x8825, 4, 1, gps_at)
ifd0 += struct.pack("<I", 0)
gps_entries = [
    (1, 2, 2, b"N\0"),
    (2, 5, 3, rationals((58, 1), (12, 1), (0, 1))),
    (3, 2, 2, b"E\0"),
    (4, 5, 3, rationals((11, 1), (54, 1), (0, 1))),
]
gps_len = 2 + len(gps_entries) * 12 + 4
gps, gps_extra = ifd(gps_entries, gps_at, gps_at + gps_len)
tiff = b"II*\0" + struct.pack("<I", ifd0_at) + ifd0 + extra0 + gps + gps_extra
app1 = b"Exif\0\0" + tiff
data = open(sys.argv[1], "rb").read()
assert data[:2] == b"\xff\xd8"
seg = b"\xff\xe1" + struct.pack(">H", len(app1) + 2) + app1
open(sys.argv[2], "wb").write(data[:2] + seg + data[2:])
