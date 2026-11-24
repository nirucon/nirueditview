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

// ------------------------------------------------------------ saving

/// `name-edited.ext`, then `-2`, never over an existing file.
pub fn edited_path(original: &Path) -> PathBuf {
    crate::pdf::copy_path(original, "edited")
}

pub fn save(img: &RgbaImage, path: &Path) -> Result<(), String> {
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
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92);
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
