//! PDF: pages are drawn by Poppler (`pdfinfo`, `pdftoppm`, in every
//! distribution), and what is added (a signature, text, a date, a tick) is
//! written with lopdf as an incremental update: the original bytes stay as
//! they are and the new objects follow them, saved as a copy.
use std::path::{Path, PathBuf};
use std::process::Command;

/// One page as Poppler shows it: its crop box in PDF points and the
/// rotation the page asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub rot: u32,
}

impl Page {
    pub fn crop_w(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn crop_h(&self) -> f64 {
        self.y1 - self.y0
    }
    /// The page as shown (points): rotated by 90 or 270, width and height swap.
    pub fn shown(&self) -> (f64, f64) {
        if self.rot % 180 == 90 {
            (self.crop_h(), self.crop_w())
        } else {
            (self.crop_w(), self.crop_h())
        }
    }

    /// A point on the shown page (points from its top-left corner) in PDF
    /// user space.
    pub fn to_pdf(self, u: f64, v: f64) -> (f64, f64) {
        let (cw, ch) = (self.crop_w(), self.crop_h());
        let (x, y) = match self.rot % 360 {
            90 => (v, u),
            180 => (cw - u, v),
            270 => (cw - v, ch - u),
            _ => (u, ch - v),
        };
        (self.x0 + x, self.y0 + y)
    }

    /// A point in PDF user space on the shown page: the inverse of `to_pdf`.
    pub fn shown_of(self, x: f64, y: f64) -> (f64, f64) {
        let (cw, ch) = (self.crop_w(), self.crop_h());
        let (x, y) = (x - self.x0, y - self.y0);
        match self.rot % 360 {
            90 => (y, x),
            180 => (cw - x, y),
            270 => (ch - y, cw - x),
            _ => (x, ch - y),
        }
    }

    /// The matrix that maps the unit square (an image, x right and y up) on
    /// to the shown rectangle (u, v, w, h), upright as it is seen.
    pub fn matrix(self, u: f64, v: f64, w: f64, h: f64) -> [f64; 6] {
        let o = self.to_pdf(u, v + h);
        let xa = self.to_pdf(u + w, v + h);
        let ya = self.to_pdf(u, v);
        [xa.0 - o.0, xa.1 - o.1, ya.0 - o.0, ya.1 - o.1, o.0, o.1]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub pages: Vec<Page>,
    pub encrypted: bool,
    pub title: String,
    /// The PDF has a form to fill in (pdfinfo says "Form: AcroForm").
    pub form: bool,
}

/// Parse `pdfinfo -f 1 -l N -box`.
pub fn parse_info(text: &str) -> Info {
    let mut count = 0usize;
    let mut encrypted = false;
    let mut title = String::new();
    let mut form = false;
    let mut pages: Vec<Page> = Vec::new();
    type Boxes = (Option<[f64; 4]>, Option<[f64; 4]>, u32);
    let mut boxes: std::collections::BTreeMap<usize, Boxes> = Default::default();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Pages:") {
            count = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Encrypted:") {
            encrypted = v.trim().starts_with("yes");
        } else if let Some(v) = line.strip_prefix("Title:") {
            title = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Form:") {
            form = v.trim() == "AcroForm";
        } else if let Some(rest) = line.strip_prefix("Page") {
            let mut it = rest.split_whitespace();
            let Some(n) = it.next().and_then(|n| n.parse::<usize>().ok()) else {
                continue;
            };
            let key = it.next().unwrap_or("");
            let nums: Vec<f64> = it.filter_map(|x| x.parse().ok()).collect();
            let e = boxes.entry(n).or_insert((None, None, 0));
            match key {
                "MediaBox:" if nums.len() == 4 => e.0 = Some([nums[0], nums[1], nums[2], nums[3]]),
                "CropBox:" if nums.len() == 4 => e.1 = Some([nums[0], nums[1], nums[2], nums[3]]),
                "rot:" if !nums.is_empty() => e.2 = (nums[0] as i64).rem_euclid(360) as u32,
                _ => {}
            }
        }
    }
    for n in 1..=count {
        let (media, crop, rot) = boxes.get(&n).cloned().unwrap_or((None, None, 0));
        let b = crop.or(media).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        pages.push(Page {
            x0: b[0].min(b[2]),
            y0: b[1].min(b[3]),
            x1: b[0].max(b[2]),
            y1: b[1].max(b[3]),
            rot,
        });
    }
    Info {
        pages,
        encrypted,
        title,
        form,
    }
}

pub fn info(path: &Path) -> Result<Info, String> {
    let out = Command::new("pdfinfo")
        .args(["-f", "1", "-l", "100000", "-box"])
        .arg(path)
        .output()
        .map_err(|e| format!("pdfinfo: {e} (install poppler)"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("pdfinfo failed").to_string());
    }
    let info = parse_info(&String::from_utf8_lossy(&out.stdout));
    if info.pages.is_empty() {
        return Err("no pages".into());
    }
    Ok(info)
}

/// A page drawn at `dpi`: RGB rows from `pdftoppm` (PPM, P6).
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

pub fn parse_ppm(data: &[u8]) -> Option<Raster> {
    // "P6" ws W ws H ws MAX single-ws data; comments are not written by pdftoppm.
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 && i < data.len() {
        while i < data.len() && data[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < data.len() && !data[i].is_ascii_whitespace() {
            i += 1;
        }
        fields.push(std::str::from_utf8(&data[start..i]).ok()?.to_string());
    }
    if fields.len() < 4 || fields[0] != "P6" || fields[3] != "255" {
        return None;
    }
    let (w, h): (u32, u32) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    let body = data.get(i + 1..)?;
    let need = (w as usize) * (h as usize) * 3;
    (body.len() >= need).then(|| Raster {
        w,
        h,
        rgb: body[..need].to_vec(),
    })
}

/// Draw page `n` (1-based) at `dpi`.
pub fn render(path: &Path, n: usize, dpi: u32) -> Result<Raster, String> {
    let out = Command::new("pdftoppm")
        .args([
            "-r",
            &dpi.to_string(),
            "-f",
            &n.to_string(),
            "-l",
            &n.to_string(),
            "-singlefile",
            "-aa",
            "yes",
            "-aaVector",
            "yes",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("pdftoppm: {e} (install poppler)"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("pdftoppm failed").to_string());
    }
    parse_ppm(&out.stdout).ok_or_else(|| "pdftoppm gave no picture".to_string())
}

// ------------------------------------------------------------ stamps

/// Something put on a page, in points on the page as shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A picture with transparency (a signature): straight RGBA.
    Picture {
        w: u32,
        h: u32,
        rgba: std::sync::Arc<Vec<u8>>,
    },
    /// One line of text in Helvetica (Latin-1), `size` points.
    Text { text: String, size: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    /// 0-based page.
    pub page: usize,
    pub u: f64,
    pub v: f64,
    pub w: f64,
    pub h: f64,
    pub kind: Kind,
    /// Ink for text: RGB 0..=1.
    pub color: [f64; 3],
}

/// Where Helvetica's baseline sits in a text stamp of `size` points, and
/// how tall the stamp is.
pub const TEXT_BASELINE: f64 = 0.92;
pub const TEXT_HEIGHT: f64 = 1.2;

/// Helvetica's widths (1/1000 em) for the printable Latin-1 range used by
/// WinAnsi, so the stamp's box fits the text as viewers will draw it.
pub fn helvetica_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| helvetica_advance(c) as f64)
        .sum::<f64>()
        * size
        / 1000.0
}

fn helvetica_advance(c: char) -> u32 {
    const ASCII: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
        722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
        556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
        500, 334, 260, 334, 584,
    ];
    let n = c as u32;
    if (32..127).contains(&n) {
        return ASCII[(n - 32) as usize] as u32;
    }
    match c {
        'å' | 'ä' | 'á' | 'à' | 'â' | 'ã' => 556,
        'Å' | 'Ä' | 'Á' | 'À' | 'Â' | 'Ã' => 667,
        'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' => 556,
        'Ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ø' => 778,
        'é' | 'è' | 'ê' | 'ë' => 556,
        'É' | 'È' | 'Ê' | 'Ë' => 667,
        'ü' | 'ú' | 'ù' | 'û' => 556,
        'Ü' | 'Ú' | 'Ù' | 'Û' => 722,
        _ => 556,
    }
}

/// WinAnsi bytes for a PDF string; characters outside Latin-1 become '?'.
/// Parentheses and backslashes are escaped.
pub fn winansi(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in text.chars() {
        let b = match c as u32 {
            0x20..=0x7e | 0xa0..=0xff => c as u32 as u8,
            0x2013 => 0x96,
            0x2014 => 0x97,
            0x2019 => 0x92,
            0x201c => 0x93,
            0x201d => 0x94,
            0x20ac => 0x80,
            _ => b'?',
        };
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

fn num(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

/// The content stream that draws the stamps of one page, after the old
/// content (which ran inside `q`, closed here with `Q`). `names` gives each
/// picture stamp's resource name, in order.
pub fn page_content(page: &Page, stamps: &[&Stamp], names: &[String], font: &str) -> Vec<u8> {
    let mut out: Vec<u8> = b"Q\n".to_vec();
    let mut pi = 0;
    for st in stamps {
        match &st.kind {
            Kind::Picture { .. } => {
                let m = page.matrix(st.u, st.v, st.w, st.h);
                out.extend_from_slice(
                    format!(
                        "q {} {} {} {} {} {} cm /{} Do Q\n",
                        num(m[0]),
                        num(m[1]),
                        num(m[2]),
                        num(m[3]),
                        num(m[4]),
                        num(m[5]),
                        names[pi]
                    )
                    .as_bytes(),
                );
                pi += 1;
            }
            Kind::Text { text, size } => {
                // Text space: one unit per point, x along the shown line.
                let m = page.matrix(st.u, st.v, 1.0, 1.0);
                let base = page.to_pdf(st.u, st.v + size * TEXT_BASELINE);
                out.extend_from_slice(
                    format!(
                        "q BT {} {} {} rg /{} {} Tf {} {} {} {} {} {} Tm (",
                        num(st.color[0]),
                        num(st.color[1]),
                        num(st.color[2]),
                        font,
                        num(*size),
                        num(m[0]),
                        num(m[1]),
                        num(m[2]),
                        num(m[3]),
                        num(base.0),
                        num(base.1)
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(&winansi(text));
                out.extend_from_slice(b") Tj ET Q\n");
            }
        }
    }
    out
}

/// Where a signed or filled-in copy goes: `name-signed.pdf`, then
/// `name-signed-2.pdf` and so on, never over an existing file.
pub fn copy_path(original: &Path, suffix: &str) -> PathBuf {
    let dir = original.parent().unwrap_or(Path::new("."));
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let ext = original
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pdf".into());
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}-{suffix}.{ext}")
        } else {
            format!("{stem}-{suffix}-{n}.{ext}")
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Add `name` -> `id` under `/Resources /category` of a page, in the new part
/// of an incremental document (an inherited or shared dictionary is copied
/// there first, so the original stays as it was).
fn add_resource(
    doc: &mut lopdf::IncrementalDocument,
    page: lopdf::ObjectId,
    category: &[u8],
    name: &str,
    id: lopdf::ObjectId,
) -> Result<(), String> {
    use lopdf::{Dictionary, Object};
    let resources = doc
        .get_or_create_resources(page)
        .and_then(Object::as_dict_mut)
        .map_err(|e| e.to_string())?;
    if !resources.has(category) {
        resources.set(category, Dictionary::new());
    }
    let entry = resources.get(category).map_err(|e| e.to_string())?.clone();
    match entry {
        Object::Reference(rid) => {
            doc.opt_clone_object_to_new_document(rid)
                .map_err(|e| e.to_string())?;
            let d = doc
                .new_document
                .get_object_mut(rid)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?;
            d.set(name, Object::Reference(id));
        }
        _ => {
            let resources = doc
                .get_or_create_resources(page)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?;
            let d = resources
                .get_mut(category)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?;
            d.set(name, Object::Reference(id));
        }
    }
    Ok(())
}

/// Write `stamps` and the changed form `fields` into a copy of `src` at
/// `dst` (an incremental update).
pub fn write_stamps(
    src: &Path,
    dst: &Path,
    pages: &[Page],
    stamps: &[Stamp],
    fields: &[crate::forms::Field],
) -> Result<(), String> {
    use lopdf::{dictionary, Object, Stream};
    let mut doc =
        lopdf::IncrementalDocument::load(src).map_err(|e| format!("cannot read the PDF: {e}"))?;
    if doc.get_prev_documents().is_encrypted() {
        return Err("the PDF is encrypted; it cannot be changed here".into());
    }
    let page_ids = doc.get_prev_documents().get_pages();
    let font_id = doc.new_document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let font_name = "NevF1".to_string();
    let mut serial = 0;
    for (index, page) in pages.iter().enumerate() {
        let mine: Vec<&Stamp> = stamps.iter().filter(|s| s.page == index).collect();
        if mine.is_empty() {
            continue;
        }
        let Some(&page_id) = page_ids.get(&(index as u32 + 1)) else {
            return Err(format!("page {} is missing", index + 1));
        };
        let mut names = Vec::new();
        for st in &mine {
            if let Kind::Picture { w, h, rgba } = &st.kind {
                let mut rgb = Vec::with_capacity((*w * *h * 3) as usize);
                let mut alpha = Vec::with_capacity((*w * *h) as usize);
                for px in rgba.chunks_exact(4) {
                    rgb.extend_from_slice(&px[..3]);
                    alpha.push(px[3]);
                }
                let smask = doc.new_document.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject",
                        "Subtype" => "Image",
                        "Width" => *w as i64,
                        "Height" => *h as i64,
                        "ColorSpace" => "DeviceGray",
                        "BitsPerComponent" => 8,
                        "Filter" => "FlateDecode",
                    },
                    deflate(&alpha),
                ));
                let image = doc.new_document.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject",
                        "Subtype" => "Image",
                        "Width" => *w as i64,
                        "Height" => *h as i64,
                        "ColorSpace" => "DeviceRGB",
                        "BitsPerComponent" => 8,
                        "Filter" => "FlateDecode",
                        "SMask" => Object::Reference(smask),
                    },
                    deflate(&rgb),
                ));
                serial += 1;
                let name = format!("NevIm{serial}");
                add_resource(&mut doc, page_id, b"XObject", &name, image)?;
                names.push(name);
            }
        }
        if mine.iter().any(|s| matches!(s.kind, Kind::Text { .. })) {
            add_resource(&mut doc, page_id, b"Font", &font_name, font_id)?;
        }
        // The old content runs inside q ... Q, so a state it leaves behind
        // (a matrix, a colour) does not move or colour the stamps.
        let before = doc
            .new_document
            .add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
        let after = doc.new_document.add_object(Stream::new(
            dictionary! { "Filter" => "FlateDecode" },
            deflate(&page_content(page, &mine, &names, &font_name)),
        ));
        doc.opt_clone_object_to_new_document(page_id)
            .map_err(|e| e.to_string())?;
        let page_dict = doc
            .new_document
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|e| e.to_string())?;
        let old: Vec<Object> = match page_dict.get(b"Contents") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(o @ Object::Reference(_)) => vec![o.clone()],
            _ => Vec::new(),
        };
        let mut contents = vec![Object::Reference(before)];
        contents.extend(old);
        contents.push(Object::Reference(after));
        page_dict.set("Contents", Object::Array(contents));
    }
    crate::forms::write(&mut doc, fields, font_id)?;
    let tmp = dst.with_extension("pdf.part");
    doc.save(&tmp)
        .map_err(|e| format!("cannot write the copy: {e}"))?;
    std::fs::rename(&tmp, dst).map_err(|e| format!("cannot write the copy: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "Title:          Form\nPages:           2\nEncrypted:       no\n\
        Page    1 size:  612 x 792 pts\nPage    1 rot:   0\nPage    2 rot:   90\n\
        Page    1 MediaBox:      0.00     0.00   612.00   792.00\n\
        Page    1 CropBox:       0.00     0.00   612.00   792.00\n\
        Page    2 MediaBox:      0.00     0.00   595.00   842.00\n\
        Page    2 CropBox:      10.00    20.00   585.00   822.00\n";

    #[test]
    fn pdfinfo_gives_pages_with_boxes_and_rotation() {
        let i = parse_info(INFO);
        assert_eq!(i.pages.len(), 2);
        assert_eq!(i.title, "Form");
        assert!(!i.encrypted);
        assert_eq!(i.pages[1].rot, 90);
        assert_eq!(i.pages[1].x0, 10.0);
        assert_eq!(i.pages[1].shown(), (802.0, 575.0));
        assert!(!i.form);
        assert!(parse_info("Form:            AcroForm\nPages: 1\n").form);
    }

    #[test]
    fn shown_points_map_to_pdf_space_for_every_rotation() {
        let p = |rot| Page {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 200.0,
            rot,
        };
        // The shown top-left corner.
        assert_eq!(p(0).to_pdf(0.0, 0.0), (0.0, 200.0));
        assert_eq!(p(90).to_pdf(0.0, 0.0), (0.0, 0.0));
        assert_eq!(p(180).to_pdf(0.0, 0.0), (100.0, 0.0));
        assert_eq!(p(270).to_pdf(0.0, 0.0), (100.0, 200.0));
        // The shown bottom-right corner is the opposite one.
        assert_eq!(p(0).to_pdf(100.0, 200.0), (100.0, 0.0));
        assert_eq!(p(90).to_pdf(200.0, 100.0), (100.0, 200.0));
        // And back again, for every rotation and an offset crop box.
        for rot in [0, 90, 180, 270] {
            let q = Page {
                x0: 10.0,
                y0: 20.0,
                x1: 110.0,
                y1: 220.0,
                rot,
            };
            let (x, y) = q.to_pdf(30.0, 40.0);
            assert_eq!(q.shown_of(x, y), (30.0, 40.0), "rot {rot}");
        }
    }

    #[test]
    fn a_picture_matrix_is_upright_on_a_plain_page() {
        let p = Page {
            x0: 0.0,
            y0: 0.0,
            x1: 600.0,
            y1: 800.0,
            rot: 0,
        };
        // 100 x 40 points, 50 from the left and 100 from the top.
        assert_eq!(
            p.matrix(50.0, 100.0, 100.0, 40.0),
            [100.0, 0.0, 0.0, 40.0, 50.0, 660.0]
        );
    }

    #[test]
    fn ppm_from_pdftoppm_is_read() {
        let mut d = b"P6\n2 1\n255\n".to_vec();
        d.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
        let r = parse_ppm(&d).unwrap();
        assert_eq!((r.w, r.h), (2, 1));
        assert_eq!(r.rgb, vec![255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn text_is_winansi_with_escapes() {
        assert_eq!(winansi("Åsa (x)\\ ✓"), b"\xc5sa \\(x\\)\\\\ ?".to_vec());
        assert!((helvetica_width("Hello", 10.0) - 22.78).abs() < 0.01);
    }

    #[test]
    fn copies_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("nev-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("form.pdf");
        assert_eq!(copy_path(&f, "signed"), dir.join("form-signed.pdf"));
        std::fs::write(dir.join("form-signed.pdf"), b"x").unwrap();
        assert_eq!(copy_path(&f, "signed"), dir.join("form-signed-2.pdf"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stamps_are_written_as_an_incremental_update() {
        use lopdf::{dictionary, Document, Object, Stream};
        let dir = std::env::temp_dir().join(format!("nev-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"0 0 1 rg 10 10 50 50 re f".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let src = dir.join("in.pdf");
        doc.save(&src).unwrap();
        let original = std::fs::read(&src).unwrap();
        let pages = vec![Page {
            x0: 0.0,
            y0: 0.0,
            x1: 300.0,
            y1: 400.0,
            rot: 0,
        }];
        let stamps = vec![
            Stamp {
                page: 0,
                u: 20.0,
                v: 30.0,
                w: 40.0,
                h: 20.0,
                kind: Kind::Picture {
                    w: 2,
                    h: 1,
                    rgba: std::sync::Arc::new(vec![0, 0, 0, 255, 0, 0, 0, 0]),
                },
                color: [0.0; 3],
            },
            Stamp {
                page: 0,
                u: 20.0,
                v: 80.0,
                w: 60.0,
                h: 14.4,
                kind: Kind::Text {
                    text: "Göteborg 2026-11-24".into(),
                    size: 12.0,
                },
                color: [0.0, 0.0, 0.0],
            },
        ];
        let dst = dir.join("out.pdf");
        write_stamps(&src, &dst, &pages, &stamps, &[]).unwrap();
        let written = std::fs::read(&dst).unwrap();
        // The original bytes are kept as they were, the update follows.
        assert!(written.len() > original.len());
        assert_eq!(&written[..original.len()], &original[..]);
        let back = Document::load(&dst).unwrap();
        let page = back.get_pages()[&1];
        let text = back.get_page_content(page);
        let text = String::from_utf8_lossy(&text);
        assert!(text.contains("/NevIm1 Do"), "{text}");
        assert!(text.contains("/NevF1 12 Tf"), "{text}");
        assert!(text.contains("0 0 1 rg 10 10 50 50 re f"), "{text}");
        let (res, _) = back.get_page_resources(page).unwrap();
        let res = res.unwrap();
        assert!(res.get(b"XObject").is_ok() && res.get(b"Font").is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
