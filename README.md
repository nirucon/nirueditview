# NIRUEDITVIEW

Pictures and PDFs for [NIRUX](https://github.com/nirucon/nirux): a fast viewer
with a gallery and a slideshow, simple picture edits, and on PDFs signatures,
text and dates, forms filled in, text found and copied, and pages turned,
moved, removed, merged and split. Rust and X11, in the NIRU theme, a few
megabytes of memory.

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
  the exposure. When a photo tells where it was taken (GPS in its EXIF or
  XMP), the details say so in the warning colour.
- **Simple edits:** rotate, mirror, crop, resize, a pen, arrows, boxes, text,
  and pixelate (faces, number plates). **Light** (`L`): brightness with `Up`
  and `Down`, contrast with `Left` and `Right`, shown before it is made.
  **Straighten** (`Z`): `Left` and `Right` turn the picture by half a degree
  (with `Shift` a tenth) over a grid, and `Enter` cuts it to the largest
  straight rectangle. `Ctrl+Z` undoes every edit. `Ctrl+S` saves a copy next
  to the original (`photo-edited.jpg`); `Ctrl+Shift+S` replaces the original
  after asking.
- **Save as** (`E`): JPG with a quality from 1 to 100 (`jpg 85`), PNG or
  WebP (written lossless), as a copy (`photo.png`, or `photo-edited.png` when
  that name is taken).
- **Privacy** (`Ctrl+L`): a copy without the location and the camera details
  (`photo-private.jpg`). EXIF, XMP, comments and embedded previews are left
  out; JPEG, PNG and WebP are copied without recompressing, and a turned
  photo stays upright.
- **Wallpaper** (`Shift+W`): the picture becomes the desktop wallpaper
  through NIRUWALLPAPER (`niruwallpaper set`).
- **PDF:** pages drawn by Poppler, scrolled as one long document. Put your
  signature on a page (draw it once with the mouse or the touchpad, or drop a
  picture of it), add text, today's date or a cross, move and resize them, and
  save a signed copy (`form-signed.pdf`). The copy is an incremental update:
  the original bytes are kept and the additions follow them.
- **Find** (`Ctrl+F`): type, and every place is lit up while the view goes to
  the first one; `Enter` goes to the next, `Shift+Enter` to the one before,
  `Esc` closes. **Copy text:** hold `Shift` and drag over it, then `Ctrl+C`.
  The text is read with Poppler (`pdftotext`) the first time it is needed.
- **Forms** (`E`): fill in the fields of a PDF form (AcroForm): text, check
  boxes, radio buttons, lists and drop-downs. Click a field or go to it with
  `Tab`, `Enter` or `Space` fills it in. `Ctrl+S` saves `form-filled.pdf`
  (an incremental update, each value with its own appearance, so every
  viewer shows it).
- **Pages** (`P`): every page as a thumbnail. `R` and `Shift+R` turn a page,
  `Delete` removes it, `Shift` and the arrows move it, `M` adds the pages of
  another PDF at the end (or drop a PDF on the window), `E` saves some pages
  as a file of their own (`3`, `2-5` or `1,4`), `Ctrl+Z` undoes, `Ctrl+S`
  saves the new order as `form-pages.pdf`. A removed page is not in the new
  file.
- **Trash** (`Delete`, asks first) puts a file in the freedesktop trash, from
  where a file manager can restore it. `Ctrl+C` copies a picture.

A signature added here is a picture on the page, the way a scanned signature
is. It is not a cryptographic (digital) signature.

Nothing of this is done when a file opens: the words of a PDF, its form and
the page thumbnails are read the first time they are asked for, so opening
a picture or a PDF is as quick as before.

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
| `Shift+W` | make the picture the wallpaper |
| `R`, `Shift+R` | rotate right, left |
| `H`, `V` | mirror, flip upside down |
| `C` | crop: drag a frame, `Enter` |
| `P`, `A`, `B`, `T`, `X` | pen, arrow, box, text, pixelate |
| `K`, `,`, `.` | ink colour, thinner, thicker |
| `Ctrl+R` | resize (`50%`, `1920x1080` or a width) |
| `L` | light: `Up` `Down` brightness, `Left` `Right` contrast, `Enter` |
| `Z` | straighten: `Left` `Right` turn, `Enter` cuts |
| `Ctrl+Z`, `Ctrl+Y` | undo, redo |
| `Ctrl+S` | save a copy |
| `E` | save as JPG (with a quality), PNG or WebP |
| `Ctrl+L` | save a copy without location and camera details |
| `Ctrl+Shift+S` | replace the original |
| PDF `S`, `Shift+S` | put your signature, draw a new one |
| PDF `T`, `D`, `X` | text, today's date, a cross |
| PDF drag, corner, arrows | move, resize, nudge; `Delete` removes |
| PDF `Ctrl+S` | save a signed (or filled) copy |
| PDF `Ctrl+F` | find: `Enter` next, `Shift+Enter` previous, `Esc` closes |
| PDF `Shift` and drag, `Ctrl+C` | choose text, copy it |
| PDF `E` | fill in the form: click or `Tab`, then `Enter` or `Space` |
| PDF `P` | pages view |
| Pages: arrows, `R`, `Shift+R` | choose, turn right, turn left |
| Pages: `Delete`, `Shift` and arrows | remove, move earlier or later |
| Pages: `M`, `E` | add a PDF at the end, save pages as a new file |
| Pages: `Ctrl+Z`, `Ctrl+S`, `Esc` | undo, save the new order, back |
| `?`, `F1` | all keys |
| `Q`, `Esc` | quit |

## Install

NIRUEDITVIEW comes with NIRUX and is set as the viewer for photos and PDFs.
On its own, from a release: put `nirueditview` in your `PATH`, and
`nirueditview.desktop` and `nirueditview.svg` where your desktop finds
entries and icons. PDF pages need Poppler (`pdftoppm`, `pdfinfo` and, to
find and copy text, `pdftotext`; the package is `poppler` on Arch,
`poppler-utils` on Debian and Void). `Shift+W` needs NIRUWALLPAPER, which
comes with NIRUX.

```sh
cargo build --release
sh tests/xvfb.sh target/release/nirueditview   # end to end in a private Xvfb (needs xclip too)
```

## Files

- `~/.local/share/nirueditview/signature.png`: your signature (mode 600).
- `~/.cache/thumbnails/large/`: thumbnails, shared with other programs.

Colours follow the NIRU theme (`~/.config/niruwm/config.toml` and the
`_NIRUWM_THEME` root property), and change live with it.

## Licence

MIT.
