# NIRUEDITVIEW 0.2.0

- PDF text: `Ctrl+F` finds text (every place lit up, `Enter` and
  `Shift+Enter` go to the next and the previous one, the view scrolls to
  it); `Shift` and a drag choose text, `Ctrl+C` copies it. The words are
  read with Poppler (`pdftotext`) only when they are first needed.
- PDF pages (`P`): a view of every page where pages are turned, removed,
  moved, another PDF is added at the end (`M` or a dropped PDF) and pages
  are saved as a file of their own (`E`, `2-5`); `Ctrl+S` saves the new
  order as `name-pages.pdf`. Undo with `Ctrl+Z`. The original is kept.
- PDF forms (`E`): text fields, check boxes, radio buttons, lists and
  drop-downs are filled in and saved as `name-filled.pdf`, an incremental
  update with an appearance for each value.
- Privacy: the details (`I`) warn when a photo has a GPS location;
  `Ctrl+L` saves a copy without location, camera and other EXIF and XMP
  details, without recompressing, upright as before.
- Save as (`E`): JPG with a quality, PNG or WebP (lossless).
- Wallpaper (`Shift+W`): through NIRUWALLPAPER (`niruwallpaper set`).
- Edits: light (`L`, brightness and contrast) and straighten (`Z`, a small
  turn with a grid, cut to a straight rectangle), with undo.
- The help (`?`) goes to two columns in a small window.
- Start-up is unchanged: nothing new is read or set up when a file opens.

# NIRUEDITVIEW 0.1.0

- First release: a fast picture and PDF viewer for NIRUX with a gallery, a
  slideshow, details from the camera, the trash and the clipboard.
- Picture edits: rotate, mirror, crop, resize, pen, arrow, box, text and
  pixelate, with undo; saved as a copy unless you choose to replace.
- PDF: pages drawn by Poppler; a signature drawn once (or a picture of it),
  text, the date and a cross put on any page, also a rotated one, and saved
  as a signed copy that keeps the original bytes (an incremental update).
