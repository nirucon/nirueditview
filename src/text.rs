//! Text with the suite's monospace font, and a sans-serif one with
//! Helvetica's measures (Liberation Sans) for text put on PDFs and pictures.
//! Glyph coverage masks are cached per (character, size).
use crate::canvas::Canvas;
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use std::collections::HashMap;

struct Mask {
    w: i32,
    h: i32,
    x: i32,
    y: i32,
    coverage: Vec<u8>,
}

pub struct Text {
    font: FontArc,
    fallback: Option<Option<FontArc>>,
    masks: HashMap<(char, u32), Option<Mask>>,
    advances: HashMap<(char, u32), f32>,
}

/// One size of the font: metrics in whole pixels.
#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub px: f32,
    pub ascent: i32,
    pub height: i32,
}

impl Text {
    pub fn new(font_path: Option<&str>) -> Result<Self, String> {
        Ok(Self {
            font: load_font(font_path, MONO)?,
            fallback: None,
            masks: HashMap::new(),
            advances: HashMap::new(),
        })
    }

    /// Liberation Sans (metric-compatible with Helvetica) or the nearest
    /// sans-serif font: what text added to a PDF will look like.
    pub fn new_sans() -> Result<Self, String> {
        Ok(Self {
            font: load_font(None, SANS)?,
            fallback: None,
            masks: HashMap::new(),
            advances: HashMap::new(),
        })
    }

    /// Draw `s` into a picture, its top at `y`.
    pub fn draw_rgba(
        &mut self,
        img: &mut image::RgbaImage,
        x: i32,
        y: i32,
        s: &str,
        size: Size,
        c: [u8; 4],
    ) -> i32 {
        let mut pen = x as f32;
        let base = y + size.ascent;
        let (iw, ih) = (img.width() as i32, img.height() as i32);
        for ch in s.chars() {
            let key = (ch, size.px.to_bits());
            if !self.masks.contains_key(&key) {
                let m = self.rasterise(ch, size.px);
                self.masks.insert(key, m);
            }
            if let Some(Some(m)) = self.masks.get(&key) {
                let (gx, gy) = (pen.round() as i32 + m.x, base + m.y);
                for my in 0..m.h {
                    for mx in 0..m.w {
                        let (px, py) = (gx + mx, gy + my);
                        if px < 0 || py < 0 || px >= iw || py >= ih {
                            continue;
                        }
                        let cov = m.coverage[(my * m.w + mx) as usize] as f32 / 255.0;
                        if cov <= 0.0 {
                            continue;
                        }
                        let p = img.get_pixel_mut(px as u32, py as u32);
                        let a = cov * c[3] as f32 / 255.0;
                        for (ch, ink) in p.0.iter_mut().zip(c).take(3) {
                            *ch = (*ch as f32 * (1.0 - a) + ink as f32 * a).round() as u8;
                        }
                        p.0[3] = (p.0[3] as f32 + (255.0 - p.0[3] as f32) * a).round() as u8;
                    }
                }
            }
            pen += self.advance(ch, size);
        }
        (pen - x as f32).ceil() as i32
    }

    pub fn size(&self, px: f32) -> Size {
        let s = self.font.as_scaled(PxScale::from(px));
        Size {
            px,
            ascent: s.ascent().ceil() as i32,
            height: s.height().ceil() as i32,
        }
    }

    fn font_for(&mut self, c: char) -> FontArc {
        if self.font.glyph_id(c).0 != 0 || c.is_ascii() {
            return self.font.clone();
        }
        let fb = self
            .fallback
            .get_or_insert_with(|| load_font(None, SYMBOLS).ok())
            .clone();
        match fb {
            Some(f) if f.glyph_id(c).0 != 0 => f,
            _ => self.font.clone(),
        }
    }

    pub fn advance(&mut self, c: char, size: Size) -> f32 {
        let key = (c, size.px.to_bits());
        if let Some(a) = self.advances.get(&key) {
            return *a;
        }
        let f = self.font_for(c);
        let s = f.as_scaled(PxScale::from(size.px));
        let a = s.h_advance(f.glyph_id(c));
        self.advances.insert(key, a);
        a
    }

    pub fn width(&mut self, s: &str, size: Size) -> i32 {
        s.chars().map(|c| self.advance(c, size)).sum::<f32>().ceil() as i32
    }

    /// The longest prefix of `s` that fits `max` pixels, with "…" when cut.
    pub fn ellipsize(&mut self, s: &str, size: Size, max: i32) -> String {
        if self.width(s, size) <= max {
            return s.to_string();
        }
        let dots = self.advance('…', size);
        let mut out = String::new();
        let mut w = 0.0;
        for c in s.chars() {
            let a = self.advance(c, size);
            if (w + a + dots).ceil() as i32 > max {
                break;
            }
            w += a;
            out.push(c);
        }
        out.push('…');
        out
    }

    /// Draw `s` with its baseline at `y + size.ascent`. Returns the width.
    pub fn draw(
        &mut self,
        cv: &mut Canvas,
        x: i32,
        y: i32,
        s: &str,
        size: Size,
        color: u32,
    ) -> i32 {
        let mut pen = x as f32;
        let base = y + size.ascent;
        for c in s.chars() {
            let key = (c, size.px.to_bits());
            if !self.masks.contains_key(&key) {
                let m = self.rasterise(c, size.px);
                self.masks.insert(key, m);
            }
            if let Some(Some(m)) = self.masks.get(&key) {
                cv.mask(
                    pen.round() as i32 + m.x,
                    base + m.y,
                    m.w,
                    m.h,
                    &m.coverage,
                    color,
                );
            }
            pen += self.advance(c, size);
        }
        (pen - x as f32).ceil() as i32
    }

    fn rasterise(&mut self, c: char, px: f32) -> Option<Mask> {
        if c == ' ' {
            return None;
        }
        let f = self.font_for(c);
        let glyph = f
            .glyph_id(c)
            .with_scale_and_position(PxScale::from(px), point(0.0, 0.0));
        let outline = f.outline_glyph(glyph)?;
        let b = outline.px_bounds();
        let (w, h) = (b.width().ceil() as i32, b.height().ceil() as i32);
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut coverage = vec![0u8; (w * h) as usize];
        outline.draw(|gx, gy, v| {
            let i = (gy as i32 * w + gx as i32) as usize;
            if i < coverage.len() {
                coverage[i] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        });
        Some(Mask {
            w,
            h,
            x: b.min.x.floor() as i32,
            y: b.min.y.floor() as i32,
            coverage,
        })
    }
}

const MONO: &str = "JetBrainsMono Nerd Font,JetBrains Mono,DejaVu Sans Mono:style=Regular";
const SYMBOLS: &str = "DejaVu Sans,Noto Sans Symbols2,Noto Sans";
const SANS: &str = "Liberation Sans,Arimo,Nimbus Sans,Helvetica,DejaVu Sans";

fn load_font(path: Option<&str>, pattern: &str) -> Result<FontArc, String> {
    if let Some(p) = path {
        let bytes = std::fs::read(p).map_err(|e| format!("font {p}: {e}"))?;
        return FontArc::try_from_vec(bytes).map_err(|e| format!("font {p}: {e}"));
    }
    let mut tried = Vec::new();
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !p.is_empty() {
            if let Ok(bytes) = std::fs::read(&p) {
                if let Ok(f) = FontArc::try_from_vec(bytes) {
                    return Ok(f);
                }
            }
            tried.push(p);
        }
    }
    for p in [
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = FontArc::try_from_vec(bytes) {
                return Ok(f);
            }
        }
        tried.push(p.to_string());
    }
    Err(format!("no usable font (tried {})", tried.join(", ")))
}
