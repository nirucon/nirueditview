//! Pictures: reading (turned upright by their EXIF orientation, every frame
//! of an animation), the edits (rotate, flip, crop, resize, pen, arrow, box,
//! text, pixelate) and saving.
use image::{
    imageops, AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgba,
    RgbaImage,
};
use std::path::{Path, PathBuf};

/// Extensions shown and opened.
pub const PICTURE_EXT: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "gif", "webp", "bmp", "tif", "tiff", "jfif",
];

pub fn is_picture(p: &Path) -> bool {
    ext(p).is_some_and(|e| PICTURE_EXT.contains(&e.as_str()))
}

pub fn is_pdf(p: &Path) -> bool {
    ext(p).is_some_and(|e| e == "pdf")
}

pub fn ext(p: &Path) -> Option<String> {
    p.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// What the info panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub taken: Option<String>,
    pub camera: Option<String>,
    pub exposure: Option<String>,
    pub place: Option<String>,
}

pub struct Picture {
    pub image: RgbaImage,
    /// Further frames of an animation with their delays (ms); the first
    /// frame is `image`.
    pub frames: Vec<(RgbaImage, u32)>,
    pub first_delay: u32,
    pub facts: Facts,
}

fn exif_facts(raw: &[u8]) -> Facts {
    let mut f = Facts::default();
    let Ok(exif) = exif::Reader::new().read_raw(raw.to_vec()) else {
        return f;
    };
    let get = |tag| {
        exif.get_field(tag, exif::In::PRIMARY)
            .map(|v| v.display_value().with_unit(&exif).to_string())
    };
    f.taken = get(exif::Tag::DateTimeOriginal).or_else(|| get(exif::Tag::DateTime));
    let make = get(exif::Tag::Make).map(|s| s.trim_matches('"').trim().to_string());
    let model = get(exif::Tag::Model).map(|s| s.trim_matches('"').trim().to_string());
    f.camera = match (make, model) {
        (Some(a), Some(b)) if b.starts_with(&a) => Some(b),
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    };
    let parts: Vec<String> = [
        get(exif::Tag::ExposureTime),
        get(exif::Tag::FNumber),
        get(exif::Tag::PhotographicSensitivity).map(|s| format!("ISO {s}")),
        get(exif::Tag::FocalLength),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !parts.is_empty() {
        f.exposure = Some(parts.join("  "));
    }
    if let (Some(lat), Some(lon)) = (get(exif::Tag::GPSLatitude), get(exif::Tag::GPSLongitude)) {
        f.place = Some(format!("{lat}  {lon}"));
    }
    f
}

pub fn load(path: &Path) -> Result<Picture, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format();
    if format == Some(ImageFormat::Gif) {
        let file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
        let dec = image::codecs::gif::GifDecoder::new(file).map_err(|e| e.to_string())?;
        let mut frames: Vec<(RgbaImage, u32)> = Vec::new();
        for f in dec.into_frames().take(2000) {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            let ms = n.checked_div(d).map_or(100, |ms| ms.max(20));
            frames.push((f.into_buffer(), ms));
        }
        if frames.is_empty() {
            return Err("no frames".into());
        }
        let (image, first_delay) = frames.remove(0);
        return Ok(Picture {
            image,
            frames,
            first_delay,
            facts: Facts::default(),
        });
    }
    let mut dec = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = dec.orientation().ok();
    let facts = dec
        .exif_metadata()
        .ok()
        .flatten()
        .map(|raw| exif_facts(&raw))
        .unwrap_or_default();
    let mut img = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(Picture {
        image: img.into_rgba8(),
        frames: Vec::new(),
        first_delay: 0,
        facts,
    })
}

/// A quick small version for the gallery: decoded at a reduced size when
/// the format allows (JPEG scales while decoding), then fitted in `max`.
pub fn thumbnail(path: &Path, max: u32) -> Result<RgbaImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut dec = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = dec.orientation().ok();
    let mut img = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    img = img.thumbnail(max, max);
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img.into_rgba8())
}

// ------------------------------------------------------------ edits

pub fn rotate(img: &RgbaImage, quarter_turns: i32) -> RgbaImage {
    match quarter_turns.rem_euclid(4) {
        1 => imageops::rotate90(img),
        2 => imageops::rotate180(img),
        3 => imageops::rotate270(img),
        _ => img.clone(),
    }
}

pub fn flip(img: &RgbaImage, horizontal: bool) -> RgbaImage {
    if horizontal {
        imageops::flip_horizontal(img)
    } else {
        imageops::flip_vertical(img)
    }
}

/// A rectangle in picture pixels, any corner order, clamped to the picture.
pub fn clamp_rect(img: &RgbaImage, a: (f64, f64), b: (f64, f64)) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let x0 = a.0.min(b.0).clamp(0.0, w).floor();
    let y0 = a.1.min(b.1).clamp(0.0, h).floor();
    let x1 = a.0.max(b.0).clamp(0.0, w).ceil();
    let y1 = a.1.max(b.1).clamp(0.0, h).ceil();
    let (cw, ch) = ((x1 - x0) as u32, (y1 - y0) as u32);
    (cw >= 2 && ch >= 2).then_some((x0 as u32, y0 as u32, cw, ch))
}

pub fn crop(img: &RgbaImage, r: (u32, u32, u32, u32)) -> RgbaImage {
    imageops::crop_imm(img, r.0, r.1, r.2, r.3).to_image()
}

pub fn resize(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    imageops::resize(img, w.max(1), h.max(1), imageops::FilterType::Lanczos3)
}

/// "50%", "1920x1080", "1920" (width, height kept in proportion).
pub fn parse_size(text: &str, w: u32, h: u32) -> Option<(u32, u32)> {
    let t = text.trim().to_ascii_lowercase().replace(' ', "");
    if let Some(p) = t.strip_suffix('%') {
        let p: f64 = p.parse().ok()?;
        if !(1.0..=1000.0).contains(&p) {
            return None;
        }
        return Some((
            ((w as f64 * p / 100.0).round() as u32).max(1),
            ((h as f64 * p / 100.0).round() as u32).max(1),
        ));
    }
    if let Some((a, b)) = t.split_once('x') {
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        return (a > 0 && b > 0 && a <= 30000 && b <= 30000).then_some((a, b));
    }
    let a: u32 = t.parse().ok()?;
    (a > 0 && a <= 30000).then(|| (a, ((h as f64 * a as f64 / w as f64).round() as u32).max(1)))
}

/// Visit the pixels a round brush of radius `r` covers along the segment,
/// with coverage for a soft edge.
pub fn brush_segment(a: (f64, f64), b: (f64, f64), r: f64, mut plot: impl FnMut(i32, i32, f32)) {
    let (minx, maxx) = (a.0.min(b.0) - r - 1.0, a.0.max(b.0) + r + 1.0);
    let (miny, maxy) = (a.1.min(b.1) - r - 1.0, a.1.max(b.1) + r + 1.0);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    for y in miny.floor() as i32..=maxy.ceil() as i32 {
        for x in minx.floor() as i32..=maxx.ceil() as i32 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = if len2 > 0.0 {
                (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                plot(x, y, cov as f32);
            }
        }
    }
}

fn blend(img: &mut RgbaImage, x: i32, y: i32, c: [u8; 4], cov: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = cov * c[3] as f32 / 255.0;
    for (ch, ink) in p.0.iter_mut().zip(c).take(3) {
        *ch = (*ch as f32 * (1.0 - a) + ink as f32 * a).round() as u8;
    }
    p.0[3] = (p.0[3] as f32 + (255.0 - p.0[3] as f32) * a).round() as u8;
}

/// A stroke through `points` without darkening where segments overlap:
/// the coverage per pixel is the strongest one any segment gives.
pub fn stroke(img: &mut RgbaImage, points: &[(f64, f64)], c: [u8; 4], width: f64) {
    let mut cover: std::collections::HashMap<(i32, i32), f32> = Default::default();
    let r = (width / 2.0).max(0.5);
    let segs: Vec<((f64, f64), (f64, f64))> = if points.len() == 1 {
        vec![(points[0], points[0])]
    } else {
        points.windows(2).map(|w| (w[0], w[1])).collect()
    };
    for (a, b) in segs {
        brush_segment(a, b, r, |x, y, cov| {
            let e = cover.entry((x, y)).or_insert(0.0);
            if cov > *e {
                *e = cov;
            }
        });
    }
    for ((x, y), cov) in cover {
        blend(img, x, y, c, cov);
    }
}

/// The points of an arrow from `a` to `b`: the shaft and the two barbs.
pub fn arrow_lines(a: (f64, f64), b: (f64, f64), width: f64) -> Vec<Vec<(f64, f64)>> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let head = (width * 4.0 + 8.0).min(len * 0.5);
    let (ux, uy) = (dx / len, dy / len);
    let barb = |s: f64| {
        let (c, sn) = (0.5f64.cos(), s * 0.5f64.sin());
        let (rx, ry) = (ux * c - uy * sn, ux * sn + uy * c);
        (b.0 - rx * head, b.1 - ry * head)
    };
    vec![vec![a, b], vec![barb(1.0), b, barb(-1.0)]]
}

pub fn rect_lines(a: (f64, f64), b: (f64, f64)) -> Vec<(f64, f64)> {
    vec![a, (b.0, a.1), b, (a.0, b.1), a]
}

/// Blocks of `block` pixels averaged: faces and numbers made unreadable.
pub fn pixelate(img: &mut RgbaImage, r: (u32, u32, u32, u32), block: u32) {
    let block = block.max(2);
    let (x0, y0, w, h) = r;
    let mut by = y0;
    while by < y0 + h {
        let mut bx = x0;
        let bh = block.min(y0 + h - by);
        while bx < x0 + w {
            let bw = block.min(x0 + w - bx);
            let mut sum = [0u64; 4];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let p = img.get_pixel(x, y).0;
                    for i in 0..4 {
                        sum[i] += p[i] as u64;
                    }
                }
            }
            let n = (bw * bh) as u64;
            let avg = Rgba([
                (sum[0] / n) as u8,
                (sum[1] / n) as u8,
                (sum[2] / n) as u8,
                (sum[3] / n) as u8,
            ]);
            for y in by..by + bh {
                for x in bx..bx + bw {
                    img.put_pixel(x, y, avg);
                }
            }
            bx += block;
        }
        by += block;
    }
}

/// Brightness and contrast (-100..=100 each) as a table for every channel
/// value; alpha is left as it is.
pub fn light_table(brightness: i32, contrast: i32) -> [u8; 256] {
    let f = ((contrast.clamp(-100, 100) + 100) as f64 / 100.0).powi(2);
    let b = brightness.clamp(-100, 100) as f64 * 2.55;
    let mut t = [0u8; 256];
    for (v, o) in t.iter_mut().enumerate() {
        *o = ((v as f64 - 127.5) * f + 127.5 + b)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    t
}

pub fn light(img: &RgbaImage, brightness: i32, contrast: i32) -> RgbaImage {
    let t = light_table(brightness, contrast);
    let mut out = img.clone();
    for p in out.pixels_mut() {
        p.0[0] = t[p.0[0] as usize];
        p.0[1] = t[p.0[1] as usize];
        p.0[2] = t[p.0[2] as usize];
    }
    out
}

/// How much of a `w` x `h` picture turned by `deg` is kept: the largest
/// rectangle of the same shape inside it, as a share of each side.
pub fn straighten_keep(w: f64, h: f64, deg: f64) -> f64 {
    let a = deg.to_radians().abs();
    let (c, s) = (a.cos(), a.sin());
    (w / (w * c + h * s)).min(h / (w * s + h * c))
}

/// Turn the picture by a small angle (degrees, clockwise) and cut it to the
/// largest rectangle of its shape that has no empty corners.
pub fn straighten(img: &RgbaImage, deg: f64) -> RgbaImage {
    if deg.abs() < 0.01 {
        return img.clone();
    }
    let (w, h) = (img.width() as f64, img.height() as f64);
    let k = straighten_keep(w, h, deg);
    let (ow, oh) = (
        ((w * k).floor() as u32).max(1),
        ((h * k).floor() as u32).max(1),
    );
    let (sn, cs) = deg.to_radians().sin_cos();
    let (cx, cy) = (w / 2.0, h / 2.0);
    let raw = img.as_raw();
    let iw = img.width() as usize;
    let px = |x: i64, y: i64, c: usize| -> f64 {
        let x = x.clamp(0, img.width() as i64 - 1) as usize;
        let y = y.clamp(0, img.height() as i64 - 1) as usize;
        raw[(y * iw + x) * 4 + c] as f64
    };
    let mut out = RgbaImage::new(ow, oh);
    for (x, y, p) in out.enumerate_pixels_mut() {
        let dx = x as f64 + 0.5 - ow as f64 / 2.0;
        let dy = y as f64 + 0.5 - oh as f64 / 2.0;
        // The source point: the output turned back.
        let sx = cx + dx * cs + dy * sn - 0.5;
        let sy = cy - dx * sn + dy * cs - 0.5;
        let (x0, y0) = (sx.floor(), sy.floor());
        let (fx, fy) = (sx - x0, sy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        for c in 0..4 {
            let top = px(x0, y0, c) * (1.0 - fx) + px(x0 + 1, y0, c) * fx;
            let bottom = px(x0, y0 + 1, c) * (1.0 - fx) + px(x0 + 1, y0 + 1, c) * fx;
            p.0[c] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

// ------------------------------------------------------------ saving

/// `name-edited.ext`, then `-2`, never over an existing file.
pub fn edited_path(original: &Path) -> PathBuf {
    crate::pdf::copy_path(original, "edited")
}

pub fn save(img: &RgbaImage, path: &Path) -> Result<(), String> {
    save_with(img, path, 92)
}

/// Save in the format the name gives; `quality` (1..=100) for JPEG. WebP is
/// written lossless.
pub fn save_with(img: &RgbaImage, path: &Path, quality: u8) -> Result<(), String> {
    let e = ext(path).unwrap_or_default();
    let tmp = path.with_file_name(format!(
        ".{}.part",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut w = std::io::BufWriter::new(file);
    let r = match e.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => {
            let rgb = DynamicImage::ImageRgba8(img.clone()).into_rgb8();
            let mut enc =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality.clamp(1, 100));
            enc.encode_image(&rgb).map_err(|e| e.to_string())
        }
        "bmp" => DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut w, ImageFormat::Bmp)
            .map_err(|e| e.to_string()),
        "tif" | "tiff" => DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut w, ImageFormat::Tiff)
            .map_err(|e| e.to_string()),
        "webp" => DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut w, ImageFormat::WebP)
            .map_err(|e| e.to_string()),
        "gif" => DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut w, ImageFormat::Gif)
            .map_err(|e| e.to_string()),
        _ => DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut w, ImageFormat::Png)
            .map_err(|e| e.to_string()),
    };
    drop(w);
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// "jpg 85", "png", "webp", "JPEG 70%": the format to save as and the JPEG
/// quality (90 when none is given).
pub fn parse_save_as(text: &str) -> Option<(&'static str, u8)> {
    let t = text.trim().to_ascii_lowercase().replace('%', "");
    let mut it = t.split_whitespace();
    let ext = match it.next()?.trim_start_matches('.') {
        "jpg" | "jpeg" => "jpg",
        "png" => "png",
        "webp" => "webp",
        _ => return None,
    };
    let q = match it.next() {
        Some(q) => q.parse::<u8>().ok().filter(|q| (1..=100).contains(q))?,
        None => 90,
    };
    it.next().is_none().then_some((ext, q))
}

/// Where a copy in another format goes: `photo.png` for `photo.jpg` when
/// that is free, else `photo-edited.png`, `-2` and so on.
pub fn converted_path(original: &Path, ext: &str) -> PathBuf {
    let same = ext_of_kind(original) == ext_of_kind(Path::new(&format!("x.{ext}")));
    let plain = original.with_extension(ext);
    if !same && !plain.exists() {
        return plain;
    }
    crate::pdf::copy_path(&plain, "edited")
}

fn ext_of_kind(p: &Path) -> String {
    match ext(p).as_deref() {
        Some("jpeg" | "jpe" | "jfif") => "jpg".into(),
        Some("tif") => "tiff".into(),
        e => e.unwrap_or("").to_string(),
    }
}

/// A photo tells where it was taken: GPS in its EXIF (`facts`), or in XMP
/// (read from the file only when asked).
pub fn xmp_location(path: &Path) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    data.windows(11).any(|w| w == b"GPSLatitude")
}

/// The EXIF orientation (1..=8) stored in a file, 1 when there is none.
pub fn orientation_of(path: &Path) -> u16 {
    let Ok(f) = std::fs::File::open(path) else {
        return 1;
    };
    let mut r = std::io::BufReader::new(f);
    exif::Reader::new()
        .read_from_container(&mut r)
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .map(|o| o as u16)
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1)
}

/// TIFF data (big-endian) with one entry: the orientation.
fn orientation_tiff(o: u16) -> Vec<u8> {
    let mut t = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    t.extend_from_slice(&o.to_be_bytes());
    t.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    t
}

/// A JPEG without its EXIF, XMP, IPTC, comments and maker segments, and
/// without anything after the picture (embedded previews): only what the
/// picture needs to be shown (JFIF, the colour profile, Adobe colour data)
/// is kept, and an orientation other than upright is written back alone.
/// The compressed picture is copied as it is, so nothing is lost.
pub fn strip_jpeg(data: &[u8], orientation: u16) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xff || data[1] != 0xd8 {
        return None;
    }
    let mut out = vec![0xff, 0xd8];
    let mut put_orientation = orientation != 1;
    let mut i = 2;
    let mut in_scan = false;
    while i + 1 < data.len() {
        if in_scan {
            // Entropy-coded data: copy up to the next real marker.
            let start = i;
            while i + 1 < data.len() {
                if data[i] == 0xff && !matches!(data[i + 1], 0x00 | 0xd0..=0xd7 | 0xff) {
                    break;
                }
                i += 1;
            }
            out.extend_from_slice(&data[start..i]);
            in_scan = false;
            continue;
        }
        if data[i] != 0xff {
            return None;
        }
        let m = data[i + 1];
        if m == 0xff {
            i += 1;
            continue;
        }
        if m == 0xd9 {
            out.extend_from_slice(&[0xff, 0xd9]);
            return Some(out);
        }
        if (0xd0..=0xd7).contains(&m) || m == 0x01 {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]) as usize;
        let seg = data.get(i..i + 2 + len)?;
        let body = &seg[4..];
        let keep = match m {
            0xe0 => true,
            0xe2 => body.starts_with(b"ICC_PROFILE\0"),
            0xee => true,
            0xe1 | 0xe3..=0xed | 0xef | 0xfe => false,
            _ => true,
        };
        if put_orientation && m != 0xe0 {
            let mut app1 = b"Exif\0\0".to_vec();
            app1.extend_from_slice(&orientation_tiff(orientation));
            out.extend_from_slice(&[0xff, 0xe1]);
            out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(&app1);
            put_orientation = false;
        }
        if keep {
            out.extend_from_slice(seg);
        }
        i += 2 + len;
        if m == 0xda {
            in_scan = true;
        }
    }
    // No end marker: the picture is cut short; keep what there is.
    out.extend_from_slice(&[0xff, 0xd9]);
    Some(out)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A PNG without EXIF, text (XMP is text in a PNG) and time chunks; an
/// orientation other than upright is written back alone.
pub fn strip_png(data: &[u8], orientation: u16) -> Option<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIG) {
        return None;
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
    while i + 8 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
        let kind = &data[i + 4..i + 8];
        let chunk = data.get(i..i + 12 + len)?;
        if kind == b"IDAT" && orientation != 1 && !out.windows(4).any(|w| w == b"eXIf") {
            let body = orientation_tiff(orientation);
            let mut c = b"eXIf".to_vec();
            c.extend_from_slice(&body);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(&c);
            out.extend_from_slice(&crc32(&c).to_be_bytes());
        }
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            out.extend_from_slice(chunk);
        }
        i += 12 + len;
        if kind == b"IEND" {
            break;
        }
    }
    Some(out)
}

/// A WebP without its EXIF and XMP chunks (and their flags).
pub fn strip_webp(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    let mut body = b"WEBP".to_vec();
    let mut i = 12;
    while i + 8 <= data.len() {
        let kind = &data[i..i + 4];
        let len = u32::from_le_bytes(data[i + 4..i + 8].try_into().ok()?) as usize;
        let padded = len + (len & 1);
        let chunk = data.get(i..(i + 8 + padded).min(data.len()))?;
        if kind == b"VP8X" && chunk.len() >= 9 {
            let mut c = chunk.to_vec();
            c[8] &= !0x0c;
            body.extend_from_slice(&c);
        } else if kind != b"EXIF" && kind != b"XMP " {
            body.extend_from_slice(chunk);
        }
        i += 8 + padded;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// Write a copy of the file at `src` without where it was taken, the camera
/// or any other EXIF and XMP details, the picture itself untouched. JPEG,
/// PNG and WebP are copied without loss; other formats are written again
/// from the upright pixels `img`.
pub fn save_private(src: &Path, img: &RgbaImage, dst: &Path) -> Result<(), String> {
    let data = std::fs::read(src).map_err(|e| e.to_string())?;
    let o = orientation_of(src);
    let stripped = match ext_of_kind(src).as_str() {
        "jpg" => strip_jpeg(&data, o),
        "png" => strip_png(&data, o),
        "webp" if o == 1 => strip_webp(&data),
        _ => None,
    };
    match stripped {
        Some(bytes) => {
            let tmp = dst.with_extension("part");
            std::fs::write(&tmp, &bytes)
                .and_then(|_| std::fs::rename(&tmp, dst))
                .map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    e.to_string()
                })
        }
        None => save_with(img, dst, 95),
    }
}

pub fn png_bytes(img: &RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = DynamicImage::ImageRgba8(img.clone()).write_to(&mut out, ImageFormat::Png);
    out.into_inner()
}

// ------------------------------------------------------------ view scaling

/// An area-average reduction to `dw` x `dh` (both smaller than the source):
/// fast enough for a 50-megapixel photo and without the shimmer of
/// skipping pixels. Pixels are 0xAARRGGBB.
pub fn reduce(src: &RgbaImage, dw: u32, dh: u32) -> Vec<u32> {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (dw, dh) = (dw.max(1) as usize, dh.max(1) as usize);
    let raw = src.as_raw();
    let mut out = vec![0u32; dw * dh];
    // Per destination column, the source columns it averages.
    let cols: Vec<(usize, usize)> = (0..dw)
        .map(|x| {
            let a = x * sw / dw;
            let b = ((x + 1) * sw / dw).max(a + 1).min(sw);
            (a, b)
        })
        .collect();
    let mut acc = vec![[0u32; 4]; dw];
    for y in 0..dh {
        let ya = y * sh / dh;
        let yb = ((y + 1) * sh / dh).max(ya + 1).min(sh);
        for a in acc.iter_mut() {
            *a = [0; 4];
        }
        for sy in ya..yb {
            let row = &raw[sy * sw * 4..(sy + 1) * sw * 4];
            for (x, &(a, b)) in cols.iter().enumerate() {
                let s = &mut acc[x];
                for sx in a..b {
                    let p = &row[sx * 4..sx * 4 + 4];
                    s[0] += p[0] as u32;
                    s[1] += p[1] as u32;
                    s[2] += p[2] as u32;
                    s[3] += p[3] as u32;
                }
            }
        }
        for (x, &(a, b)) in cols.iter().enumerate() {
            let n = ((b - a) * (yb - ya)) as u32;
            let s = acc[x];
            out[y * dw + x] =
                ((s[3] / n) << 24) | ((s[0] / n) << 16) | ((s[1] / n) << 8) | (s[2] / n);
        }
    }
    out
}

pub fn argb(src: &RgbaImage) -> Vec<u32> {
    src.pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_read_as_percent_box_or_width() {
        assert_eq!(parse_size("50%", 4000, 3000), Some((2000, 1500)));
        assert_eq!(parse_size("1920x1080", 4000, 3000), Some((1920, 1080)));
        assert_eq!(parse_size("1000", 4000, 3000), Some((1000, 750)));
        assert_eq!(parse_size("0%", 4000, 3000), None);
        assert_eq!(parse_size("abc", 4000, 3000), None);
    }

    #[test]
    fn reduce_averages_blocks() {
        let mut img = RgbaImage::new(4, 2);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            };
        }
        let out = reduce(&img, 2, 1);
        assert_eq!(out, vec![0xffff0000, 0xff0000ff]);
    }

    #[test]
    fn edits_turn_flip_crop_and_pixelate() {
        let mut img = RgbaImage::from_pixel(4, 2, Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        let r = rotate(&img, 1);
        assert_eq!(r.dimensions(), (2, 4));
        assert_eq!(r.get_pixel(1, 0).0[0], 255);
        let f = flip(&img, true);
        assert_eq!(f.get_pixel(3, 0).0[0], 255);
        let c = crop(&img, clamp_rect(&img, (0.0, 0.0), (2.0, 2.0)).unwrap());
        assert_eq!(c.dimensions(), (2, 2));
        assert!(clamp_rect(&img, (1.0, 1.0), (1.5, 1.2)).is_none());
        pixelate(&mut img, (0, 0, 2, 2), 2);
        assert_eq!(img.get_pixel(1, 1).0[0], 63);
    }

    #[test]
    fn a_stroke_covers_its_path_once() {
        let mut img = RgbaImage::from_pixel(20, 20, Rgba([255, 255, 255, 255]));
        stroke(
            &mut img,
            &[(2.0, 10.0), (10.0, 10.0), (18.0, 10.0)],
            [255, 0, 0, 255],
            4.0,
        );
        assert_eq!(img.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(10, 2).0, [255, 255, 255, 255]);
    }

    #[test]
    fn light_and_straighten() {
        let t = light_table(0, 0);
        assert!((0..256).all(|v| t[v] == v as u8));
        assert_eq!(light_table(100, 0)[0], 255);
        assert_eq!(light_table(0, -100)[10], 128);
        assert!(light_table(0, 50)[200] > 200 && light_table(0, 50)[50] < 50);
        let img = RgbaImage::from_pixel(200, 100, Rgba([10, 20, 30, 255]));
        let l = light(&img, 10, 0);
        assert_eq!(l.get_pixel(0, 0).0, [36, 46, 56, 255]);
        assert_eq!(straighten(&img, 0.0).dimensions(), (200, 100));
        let s = straighten(&img, 5.0);
        let k = straighten_keep(200.0, 100.0, 5.0);
        assert!(k < 1.0 && k > 0.8);
        assert_eq!(s.dimensions(), ((200.0 * k) as u32, (100.0 * k) as u32));
        // No empty corners: every pixel is the picture's colour.
        assert!(s.pixels().all(|p| p.0 == [10, 20, 30, 255]));
        // A turned line ends up level: a dark row tilted by 3 degrees.
        let mut tilted = RgbaImage::from_pixel(400, 200, Rgba([255, 255, 255, 255]));
        for x in 0..400 {
            let y = (100.0 + (x as f64 - 200.0) * 3f64.to_radians().tan()).round() as u32;
            for d in 0..3 {
                tilted.put_pixel(x, y + d - 1, Rgba([0, 0, 0, 255]));
            }
        }
        let level = straighten(&tilted, -3.0);
        let row = |x: u32| (0..level.height()).find(|y| level.get_pixel(x, *y).0[0] < 100);
        let (a, b) = (row(20).unwrap(), row(level.width() - 20).unwrap());
        assert!(a.abs_diff(b) <= 1, "{a} {b}");
    }

    #[test]
    fn save_as_takes_a_format_and_a_quality() {
        assert_eq!(parse_save_as("jpg 85"), Some(("jpg", 85)));
        assert_eq!(parse_save_as("JPEG 70%"), Some(("jpg", 70)));
        assert_eq!(parse_save_as("png"), Some(("png", 90)));
        assert_eq!(parse_save_as(".webp"), Some(("webp", 90)));
        assert_eq!(parse_save_as("gif"), None);
        assert_eq!(parse_save_as("jpg 0"), None);
        assert_eq!(parse_save_as("jpg 101"), None);
        let dir = std::env::temp_dir().join(format!("nev-as-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.jpg");
        assert_eq!(converted_path(&a, "png"), dir.join("a.png"));
        assert_eq!(converted_path(&a, "jpg"), dir.join("a-edited.jpg"));
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        assert_eq!(converted_path(&a, "png"), dir.join("a-edited.png"));
        // A lower quality makes a smaller file.
        let mut img = RgbaImage::new(64, 64);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgba([(x * 4) as u8, (y * 4) as u8, ((x ^ y) * 4) as u8, 255]);
        }
        save_with(&img, &dir.join("hi.jpg"), 95).unwrap();
        save_with(&img, &dir.join("lo.jpg"), 30).unwrap();
        let size = |n: &str| std::fs::metadata(dir.join(n)).unwrap().len();
        assert!(size("lo.jpg") < size("hi.jpg"));
        save_with(&img, &dir.join("l.webp"), 50).unwrap();
        assert_eq!(
            load(&dir.join("l.webp")).unwrap().image,
            img,
            "WebP is lossless"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A JPEG with EXIF (camera, GPS and an orientation), XMP with GPS and
    /// a comment, the way a phone writes one.
    pub fn phone_jpeg(orientation: u16) -> Vec<u8> {
        let img = RgbaImage::from_pixel(40, 20, Rgba([200, 30, 30, 255]));
        let rgb = DynamicImage::ImageRgba8(img).into_rgb8();
        let mut plain = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90)
            .encode_image(&rgb)
            .unwrap();
        use exif::experimental::Writer;
        use exif::{Field, In, Rational, Tag, Value};
        let f = |tag, value| Field {
            tag,
            ifd_num: In::PRIMARY,
            value,
        };
        let fields = [
            f(Tag::Make, Value::Ascii(vec![b"PhoneCo".to_vec()])),
            f(Tag::Model, Value::Ascii(vec![b"Phone 9".to_vec()])),
            f(Tag::Orientation, Value::Short(vec![orientation])),
            f(Tag::GPSLatitudeRef, Value::Ascii(vec![b"N".to_vec()])),
            f(
                Tag::GPSLatitude,
                Value::Rational(vec![Rational { num: 58, denom: 1 }; 3]),
            ),
            f(Tag::GPSLongitudeRef, Value::Ascii(vec![b"E".to_vec()])),
            f(
                Tag::GPSLongitude,
                Value::Rational(vec![Rational { num: 11, denom: 1 }; 3]),
            ),
        ];
        let mut w = Writer::new();
        for x in &fields {
            w.push_field(x);
        }
        let mut tiff = std::io::Cursor::new(Vec::new());
        w.write(&mut tiff, false).unwrap();
        let seg = |m: u8, body: &[u8]| {
            let mut s = vec![0xff, m];
            s.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            s.extend_from_slice(body);
            s
        };
        let mut exif_body = b"Exif\0\0".to_vec();
        exif_body.extend_from_slice(&tiff.into_inner());
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta><rdf:Description exif:GPSLatitude=\"58,1N\"/></x:xmpmeta>";
        let mut out = plain[..2].to_vec();
        out.extend(seg(0xe1, &exif_body));
        out.extend(seg(0xe1, xmp));
        out.extend(seg(0xfe, b"taken at home"));
        out.extend_from_slice(&plain[2..]);
        // A preview after the picture, as some cameras add.
        out.extend_from_slice(b"\xff\xd8 preview GPSLatitude \xff\xd9");
        out
    }

    #[test]
    fn a_private_copy_has_no_location_or_camera_and_the_same_picture() {
        let dir = std::env::temp_dir().join(format!("nev-priv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for o in [1u16, 6] {
            let src = dir.join(format!("phone{o}.jpg"));
            std::fs::write(&src, phone_jpeg(o)).unwrap();
            let pic = load(&src).unwrap();
            assert!(pic.facts.place.is_some() && pic.facts.camera.is_some());
            assert!(xmp_location(&src));
            let dst = dir.join(format!("phone{o}-private.jpg"));
            save_private(&src, &pic.image, &dst).unwrap();
            let back = load(&dst).unwrap();
            assert_eq!(back.facts.place, None);
            assert_eq!(back.facts.camera, None);
            assert!(!xmp_location(&dst), "no XMP, comment or preview with GPS");
            let raw = std::fs::read(&dst).unwrap();
            assert!(!raw.windows(13).any(|w| w == b"taken at home"));
            // Upright as before: turned photos keep their orientation.
            assert_eq!(
                back.image.dimensions(),
                pic.image.dimensions(),
                "orientation {o}"
            );
            assert_eq!(orientation_of(&dst), o);
            // The compressed picture is the same bytes.
            let orig = std::fs::read(&src).unwrap();
            let sos = |d: &[u8]| d.windows(2).position(|w| w == [0xff, 0xda]).unwrap();
            let tail = &raw[sos(&raw)..];
            assert_eq!(&orig[sos(&orig)..sos(&orig) + tail.len()], tail);
        }
        // PNG: text chunks go.
        let img = RgbaImage::from_pixel(4, 4, Rgba([1, 2, 3, 255]));
        let png = dir.join("p.png");
        save(&img, &png).unwrap();
        let mut data = std::fs::read(&png).unwrap();
        let mut chunk = b"tEXtComment\0GPSLatitude 58".to_vec();
        let crc = crc32(&chunk);
        let mut c = ((chunk.len() - 4) as u32).to_be_bytes().to_vec();
        c.append(&mut chunk);
        c.extend_from_slice(&crc.to_be_bytes());
        data.splice(33..33, c);
        std::fs::write(&png, &data).unwrap();
        assert!(xmp_location(&png));
        let out = dir.join("p-private.png");
        save_private(&png, &img, &out).unwrap();
        assert!(!xmp_location(&out));
        assert_eq!(load(&out).unwrap().image, img);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_keeps_the_format_of_the_name() {
        let dir = std::env::temp_dir().join(format!("nev-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = RgbaImage::from_pixel(8, 6, Rgba([10, 200, 30, 255]));
        for name in ["a.png", "a.jpg", "a.webp", "a.bmp"] {
            let p = dir.join(name);
            save(&img, &p).unwrap();
            let back = load(&p).unwrap();
            assert_eq!(back.image.dimensions(), (8, 6), "{name}");
        }
        assert_eq!(edited_path(&dir.join("a.png")), dir.join("a-edited.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
