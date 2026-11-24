# NIRUEDITVIEW

Pictures and PDFs for [NIRUX](https://github.com/nirucon/nirux): a fast viewer
with a gallery and a slideshow, simple picture edits, and signatures, text and
dates put on PDFs. Rust and X11, in the NIRU theme, a few megabytes of memory.

It is the default viewer for photos and PDFs in NIRUX.

## What it does

- **Look through a folder.** Open one picture and the whole folder follows, in
  natural order (`img2` before `img10`), or by date or size (`O`). The next
  and previous pictures are decoded in the background, so turning is instant.
  Photos are shown upright from their EXIF orientation; animated GIFs play.
- **Gallery** (`G`) with thumbnails shared with NIRUFM and other file
  managers (`~/.cache/thumbnails/large`).
- **Slideshow** (`S` or `F5`): full screen, `Space` pauses, `+` and `-` set
  the pace.
- **Details** (`I`): size, pixels, when the photo was taken, the camera and
  the exposure.
- **Simple edits:** rotate, mirror, crop, resize, a pen, arrows, boxes, text,
  and pixelate (faces, number plates). `Ctrl+Z` undoes. `Ctrl+S` saves a copy
  next to the original (`photo-edited.jpg`); `Ctrl+Shift+S` replaces the
  original after asking.
- **PDF:** pages drawn by Poppler, scrolled as one long document. Put your
  signature on a page (draw it once with the mouse or the touchpad, or drop a
  picture of it), add text, today's date or a cross, move and resize them, and
  save a signed copy (`form-signed.pdf`). The copy is an incremental update:
  the original bytes are kept and the additions follow them.
- **Trash** (`Delete`, asks first) puts a file in the freedesktop trash, from
  where a file manager can restore it. `Ctrl+C` copies a picture.

A signature added here is a picture on the page, the way a scanned signature
is. It is not a cryptographic (digital) signature.

## Keys

| Key | What it does |
|---|---|
| `Right`, `Space`, `PgDn` | next picture, or next page of a PDF |
| `Left`, `Backspace`, `PgUp` | previous |
| `[` `]` | previous and next file |
| `Home`, `End` | first and last |
| wheel, `+`, `-` | zoom at the pointer (PDF: the wheel scrolls, `Ctrl` and the wheel zoom) |
| `0`, `1`, `W` | fit, actual size, fit the width |
| drag | move around; a click at the left or right edge turns the page |
| double click, `F`, `F11` | full screen |
| `G` | gallery |
| `S`, `F5` | slideshow |
| `I` | details |
| `O` | sort by name, date or size |
| `Delete` | move to the trash |
| `Ctrl+C` | copy the picture |
| `R`, `Shift+R` | rotate right, left |
| `H`, `V` | mirror, flip upside down |
| `C` | crop: drag a frame, `Enter` |
| `P`, `A`, `B`, `T`, `X` | pen, arrow, box, text, pixelate |
| `K`, `,`, `.` | ink colour, thinner, thicker |
| `Ctrl+R` | resize (`50%`, `1920x1080` or a width) |
| `Ctrl+Z`, `Ctrl+Y` | undo, redo |
| `Ctrl+S` | save a copy |
| `Ctrl+Shift+S` | replace the original |
| PDF `S`, `Shift+S` | put your signature, draw a new one |
| PDF `T`, `D`, `X` | text, today's date, a cross |
| PDF drag, corner, arrows | move, resize, nudge; `Delete` removes |
| PDF `Ctrl+S` | save a signed copy |
| `?`, `F1` | all keys |
| `Q`, `Esc` | quit |

## Install

NIRUEDITVIEW comes with NIRUX and is set as the viewer for photos and PDFs.
On its own, from a release: put `nirueditview` in your `PATH`, and
`nirueditview.desktop` and `nirueditview.svg` where your desktop finds
entries and icons. PDF pages need Poppler (`pdftoppm`, `pdfinfo`; the
package is `poppler` on Arch, `poppler-utils` on Debian and Void).

```sh
cargo build --release
sh tests/xvfb.sh target/release/nirueditview   # end to end in a private Xvfb
```

## Files

- `~/.local/share/nirueditview/signature.png`: your signature (mode 600).
- `~/.cache/thumbnails/large/`: thumbnails, shared with other programs.

Colours follow the NIRU theme (`~/.config/niruwm/config.toml` and the
`_NIRUWM_THEME` root property), and change live with it.

## Licence

MIT.
