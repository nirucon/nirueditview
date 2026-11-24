//! Drawing: the picture or the pages, the bars, the gallery, the panels.
use crate::app::{App, Doc, Drag, Tool, INKS};
use crate::canvas::Rect;
use crate::jobs::Job;
use crate::{pdf, picture, theme};

const HELP: &[(&str, &str)] = &[
    ("Right, Space, PgDn", "next picture (or page)"),
    ("Left, Backspace, PgUp", "previous"),
    ("[  ]", "previous and next file"),
    ("Home, End", "first and last"),
    ("Wheel, + -", "zoom (PDF: wheel scrolls, Ctrl+wheel zooms)"),
    ("0   1   W", "fit, actual size, fit width"),
    ("Drag", "move around; a click at the edge turns the page"),
    ("Double click, F", "full screen"),
    ("G", "gallery"),
    ("S, F5", "slideshow (Space pauses, + - pace)"),
    ("I", "details: size, date, camera"),
    ("O", "sort by name, date or size"),
    ("Delete", "move to the trash (asks first)"),
    ("Ctrl+C", "copy the picture"),
    ("", ""),
    ("R  Shift+R", "rotate right, left"),
    ("H  V", "mirror, flip upside down"),
    ("C", "crop: drag a frame, Enter"),
    ("P  A  B", "pen, arrow, box"),
    ("T", "text"),
    ("X", "pixelate (faces, numbers)"),
    ("K   ,  .", "ink colour, thinner, thicker"),
    ("Ctrl+R", "resize"),
    ("Ctrl+Z  Ctrl+Y", "undo, redo"),
    ("Ctrl+S", "save a copy (name-edited)"),
    ("Ctrl+Shift+S", "replace the original (asks first)"),
    ("", ""),
    ("PDF: S", "put your signature (Shift+S: a new one)"),
    ("PDF: T  D  X", "text, today's date, a cross"),
    ("PDF: drag, corner", "move, resize; Delete removes"),
    ("PDF: Ctrl+S", "save a signed copy (name-signed.pdf)"),
    ("Q, Esc", "quit"),
];

impl App {
    pub fn draw(&mut self) {
        let p = self.pal;
        let (w, h) = (self.cv.w, self.cv.h);
        let ground = if self.x.fullscreen || self.slides.is_some() {
            0x000000
        } else {
            theme::blend(p.background, 0x000000, 0.25)
        };
        self.cv.reset_clip();
        self.cv.fill(Rect::new(0, 0, w, h), ground);
        if self.gallery {
            self.draw_gallery();
        } else {
            let v = self.view();
            self.cv.set_clip(v);
            match &self.doc {
                Doc::Pic(_) => self.draw_picture(),
                Doc::Pdf(_) => self.draw_pdf(),
                Doc::Empty(m) => {
                    let m = m.clone();
                    self.centred(&m, p.muted);
                }
                Doc::Loading(_) => self.centred("…", p.muted),
            }
            self.cv.reset_clip();
        }
        if self.bar_h() > 0 {
            self.draw_bars();
        } else if let Some((m, _, err)) = self.message.clone() {
            self.toast(&m, err);
        }
        if self.info {
            self.draw_info();
        }
        if self.pad.is_some() {
            self.draw_pad();
        }
        if self.help {
            self.draw_help();
        }
    }

    fn centred(&mut self, s: &str, color: u32) {
        let size = self.ui.size(self.font_px);
        let v = self.view();
        let s = self.ui.ellipsize(s, size, v.w - 40);
        let tw = self.ui.width(&s, size);
        self.ui.draw(
            &mut self.cv,
            v.x + (v.w - tw) / 2,
            v.y + (v.h - size.height) / 2,
            &s,
            size,
            color,
        );
    }

    fn checker(&self) -> (u32, u32) {
        (
            theme::blend(self.pal.background, 0x808080, 0.35),
            theme::blend(self.pal.background, 0x808080, 0.55),
        )
    }

    fn draw_picture(&mut self) {
        let Doc::Pic(d) = &self.doc else { return };
        let img = d.shown();
        let generation = d.generation;
        let (iw, ih) = img.dimensions();
        let z = self.zoom;
        let (sw, sh) = (
            ((iw as f64 * z).round() as u32).max(1),
            ((ih as f64 * z).round() as u32).max(1),
        );
        let v = self.view();
        let screen = Rect::new(
            self.ox.round() as i32,
            self.oy.round() as i32,
            sw as i32,
            sh as i32,
        );
        let vis = screen.intersect(&v);
        let checker = self.checker();
        if vis.w <= 0 || vis.h <= 0 {
            return;
        }
        if z < 1.0 {
            let key = (generation, z.to_bits());
            if self.scaled.as_ref().is_none_or(|s| (s.0, s.1) != key) {
                let px = picture::reduce(img, sw, sh);
                self.scaled = Some((key.0, key.1, sw, sh, px));
            }
            let (_, _, cw, _, px) = self.scaled.as_ref().unwrap();
            let (cw, sx0, sy0) = (*cw as usize, vis.x - screen.x, vis.y - screen.y);
            let mut buf = Vec::with_capacity((vis.w * vis.h) as usize);
            for y in 0..vis.h {
                let row = (sy0 + y) as usize * cw;
                buf.extend_from_slice(&px[row + sx0 as usize..row + (sx0 + vis.w) as usize]);
            }
            self.cv
                .picture(vis.x, vis.y, vis.w, vis.h, &buf, vis.w as usize, checker);
        } else {
            if self.full.as_ref().is_none_or(|f| f.0 != generation) {
                self.full = Some((generation, picture::argb(img)));
            }
            let full = &self.full.as_ref().unwrap().1;
            let mut buf = Vec::with_capacity((vis.w * vis.h) as usize);
            let cols: Vec<usize> = (0..vis.w)
                .map(|x| {
                    (((vis.x + x) as f64 + 0.5 - self.ox) / z)
                        .floor()
                        .clamp(0.0, iw as f64 - 1.0) as usize
                })
                .collect();
            for y in 0..vis.h {
                let sy = (((vis.y + y) as f64 + 0.5 - self.oy) / z)
                    .floor()
                    .clamp(0.0, ih as f64 - 1.0) as usize;
                let row = sy * iw as usize;
                buf.extend(cols.iter().map(|c| full[row + c]));
            }
            self.cv
                .picture(vis.x, vis.y, vis.w, vis.h, &buf, vis.w as usize, checker);
        }
        // What is being drawn, before it is put into the picture.
        let ink = INKS[self.ink].0;
        let width = self.width;
        let accent = self.pal.mark();
        match &self.drag {
            Drag::Line(points) if self.tool == Tool::Pen => {
                let pts: Vec<(f64, f64)> = points.iter().map(|p| self.to_screen(*p)).collect();
                self.cv.stroke(&pts, ink, width);
            }
            Drag::Rect(a, b) => {
                let (a, b) = (self.to_screen(*a), self.to_screen(*b));
                match self.tool {
                    Tool::Arrow => {
                        for line in picture::arrow_lines(a, b, width) {
                            self.cv.stroke(&line, ink, width);
                        }
                    }
                    Tool::Box => self.cv.stroke(&picture::rect_lines(a, b), ink, width),
                    _ => {
                        let r = rect_of(a, b);
                        self.cv.tint(r, accent, 50);
                        self.cv.dashed(r, accent, 4);
                    }
                }
            }
            _ => {}
        }
        if let Some((a, b)) = self.crop {
            let r = rect_of(self.to_screen(a), self.to_screen(b));
            // Darken what is cut away.
            for out in [
                Rect::new(v.x, v.y, v.w, r.y - v.y),
                Rect::new(v.x, r.y + r.h, v.w, v.y + v.h - r.y - r.h),
                Rect::new(v.x, r.y, r.x - v.x, r.h),
                Rect::new(r.x + r.w, r.y, v.x + v.w - r.x - r.w, r.h),
            ] {
                self.cv.tint(out, 0x000000, 150);
            }
            self.cv.dashed(r, accent, 4);
        }
    }

    fn draw_pdf(&mut self) {
        let Doc::Pdf(d) = &self.doc else { return };
        let v = self.view();
        let n = d.info.pages.len();
        let mut draws = Vec::new();
        for i in 0..n {
            let (x, y, w, h) = self.page_rect(d, i);
            if y + h < v.y as f64 || y > (v.y + v.h) as f64 {
                continue;
            }
            draws.push((i, x, y, w, h));
        }
        let shadow = theme::blend(self.pal.background, 0x000000, 0.6);
        for (i, x, y, w, h) in draws {
            let r = Rect::new(
                x.round() as i32,
                y.round() as i32,
                w.round() as i32,
                h.round() as i32,
            );
            self.cv.fill(Rect::new(r.x + 3, r.y + 3, r.w, r.h), shadow);
            self.cv.fill(r, 0xffffff);
            let Doc::Pdf(d) = &self.doc else { return };
            if let Some(raster) = d.pages.get(&i) {
                let vis = r.intersect(&v);
                if vis.w > 0 && vis.h > 0 {
                    let (rw, rh) = (raster.w as f64, raster.h as f64);
                    let mut buf = Vec::with_capacity((vis.w * vis.h) as usize);
                    let cols: Vec<usize> = (0..vis.w)
                        .map(|cx| {
                            ((((vis.x + cx - r.x) as f64 + 0.5) * rw / r.w as f64) as usize)
                                .min(raster.w as usize - 1)
                        })
                        .collect();
                    for cy in 0..vis.h {
                        let sy = ((((vis.y + cy - r.y) as f64 + 0.5) * rh / r.h as f64) as usize)
                            .min(raster.h as usize - 1);
                        let row = sy * raster.w as usize;
                        buf.extend(cols.iter().map(|c| raster.px[row + c]));
                    }
                    self.cv.picture(
                        vis.x,
                        vis.y,
                        vis.w,
                        vis.h,
                        &buf,
                        vis.w as usize,
                        (0xffffff, 0xffffff),
                    );
                }
            }
            let Doc::Pdf(d) = &self.doc else { return };
            let stamps: Vec<(usize, pdf::Stamp)> = d
                .stamps
                .iter()
                .enumerate()
                .filter(|(_, s)| s.page == i)
                .map(|(k, s)| (k, s.clone()))
                .collect();
            let sel = d.sel;
            for (k, s) in stamps {
                self.draw_stamp(&s, x, y, sel == Some(k));
            }
            if let Some(pl) = &self.placing {
                if pl.stamp.page == i {
                    let s = pl.stamp.clone();
                    self.draw_stamp(&s, x, y, true);
                }
            }
        }
    }

    fn draw_stamp(&mut self, s: &pdf::Stamp, px: f64, py: f64, selected: bool) {
        let z = self.zoom;
        let r = Rect::new(
            (px + s.u * z).round() as i32,
            (py + s.v * z).round() as i32,
            (s.w * z).round().max(1.0) as i32,
            (s.h * z).round().max(1.0) as i32,
        );
        match &s.kind {
            pdf::Kind::Picture { w, h, rgba } => {
                let (iw, ih) = (*w as usize, *h as usize);
                for y in 0..r.h {
                    let sy = ((y as f64 + 0.5) * ih as f64 / r.h as f64) as usize;
                    let sy = sy.min(ih - 1);
                    for x in 0..r.w {
                        let sx = (((x as f64 + 0.5) * iw as f64 / r.w as f64) as usize).min(iw - 1);
                        let i = (sy * iw + sx) * 4;
                        let a = rgba[i + 3];
                        if a > 0 {
                            let c = ((rgba[i] as u32) << 16)
                                | ((rgba[i + 1] as u32) << 8)
                                | rgba[i + 2] as u32;
                            self.cv.put(r.x + x, r.y + y, c, a as f32 / 255.0);
                        }
                    }
                }
            }
            pdf::Kind::Text { text, size } => {
                let px_size = (*size * z) as f32;
                if px_size >= 3.0 {
                    let fs = self.sans.size(px_size);
                    let top =
                        (py + s.v * z + pdf::TEXT_BASELINE * size * z).round() as i32 - fs.ascent;
                    let c = ((s.color[0] * 255.0) as u32) << 16
                        | ((s.color[1] * 255.0) as u32) << 8
                        | (s.color[2] * 255.0) as u32;
                    let text = text.clone();
                    self.sans.draw(&mut self.cv, r.x, top, &text, fs, c);
                }
            }
        }
        if selected {
            let a = self.pal.mark();
            self.cv.dashed(r.inset(-2), a, 3);
            self.cv
                .fill(Rect::new(r.x + r.w - 3, r.y + r.h - 3, 7, 7), a);
        }
    }

    fn draw_gallery(&mut self) {
        let p = self.pal;
        let v = self.view();
        let (cw, ch) = self.gallery_cell();
        let cols = self.gallery_cols();
        let left = v.x + (v.w - cols * (cw + 12) + 12) / 2;
        let size = self.ui.size(self.font_px);
        self.cv.set_clip(v);
        let rows = (self.list.len() as i32 + cols - 1) / cols;
        let max_scroll = (rows * (ch + 12) + 24 - v.h).max(0);
        self.gallery_scroll = self.gallery_scroll.clamp(0, max_scroll);
        for (i, path) in self.list.clone().iter().enumerate() {
            let (c, r) = (i as i32 % cols, i as i32 / cols);
            let x = left + c * (cw + 12);
            let y = v.y + 12 + r * (ch + 12) - self.gallery_scroll;
            if y + ch < v.y || y > v.y + v.h {
                continue;
            }
            let cell = Rect::new(x, y, cw, ch);
            if i == self.index {
                self.cv.fill(cell.inset(-4), p.selected);
                self.cv.frame(cell.inset(-4), p.mark());
            }
            let box_r = Rect::new(x, y, cw, cw);
            self.cv.fill(box_r, p.surface);
            match self.thumbs.get(path) {
                Some(Some((tw, th, px))) => {
                    let (tw, th) = (*tw as i32, *th as i32);
                    let px = px.clone();
                    self.cv.picture(
                        x + (cw - tw) / 2,
                        y + (cw - th) / 2,
                        tw,
                        th,
                        &px,
                        tw as usize,
                        self.checker(),
                    );
                }
                Some(None) => {
                    let s = if picture::is_pdf(path) { "PDF" } else { "?" };
                    let tw = self.ui.width(s, size);
                    self.ui.draw(
                        &mut self.cv,
                        x + (cw - tw) / 2,
                        y + cw / 2 - size.height / 2,
                        s,
                        size,
                        p.muted,
                    );
                }
                None => {
                    self.jobs.ask(Job::Thumb { path: path.clone() });
                }
            }
            let name = crate::app::name_of(path);
            let name = self.ui.ellipsize(&name, size, cw);
            let nw = self.ui.width(&name, size);
            let color = if i == self.index {
                p.foreground
            } else {
                p.muted
            };
            self.ui.draw(
                &mut self.cv,
                x + (cw - nw) / 2,
                y + cw + 4,
                &name,
                size,
                color,
            );
        }
        self.cv.reset_clip();
    }

    fn draw_bars(&mut self) {
        let p = self.pal;
        let b = self.bar_h();
        let (w, h) = (self.cv.w, self.cv.h);
        let size = self.ui.size(self.font_px);
        let ty = (b - size.height) / 2;
        self.cv.fill(Rect::new(0, 0, w, b), p.background);
        self.cv.fill(Rect::new(0, b - 1, w, 1), p.border);
        self.cv.fill(Rect::new(0, h - b, w, b), p.background);
        self.cv.fill(Rect::new(0, h - b, w, 1), p.border);
        // Top: the name, where in the folder, size and zoom.
        let (name, modified) = match &self.doc {
            Doc::Pic(d) => (crate::app::name_of(&d.path), d.modified),
            Doc::Pdf(d) => (crate::app::name_of(&d.path), d.modified()),
            Doc::Loading(pth) => (crate::app::name_of(pth), false),
            Doc::Empty(_) => ("NIRUEDITVIEW".to_string(), false),
        };
        let right = match &self.doc {
            Doc::Pic(d) => format!(
                "{} x {}   {}%",
                d.image.width(),
                d.image.height(),
                (self.zoom * 100.0).round()
            ),
            Doc::Pdf(d) => format!(
                "page {} of {}   {}%",
                self.current_page(d) + 1,
                d.info.pages.len(),
                (self.zoom * 100.0).round()
            ),
            _ => String::new(),
        };
        let count = if self.list.len() > 1 {
            format!("{} / {}", self.index + 1, self.list.len())
        } else {
            String::new()
        };
        let rw = self.ui.width(&right, size);
        let cw = self.ui.width(&count, size);
        let shown = self.ui.ellipsize(&name, size, (w - rw - cw - 60).max(40));
        let nw = self
            .ui
            .draw(&mut self.cv, 10, ty, &shown, size, p.foreground);
        if modified {
            self.ui
                .draw(&mut self.cv, 10 + nw + 6, ty, "*", size, p.warning);
        }
        self.ui
            .draw(&mut self.cv, (w - cw) / 2, ty, &count, size, p.muted);
        self.ui
            .draw(&mut self.cv, w - rw - 10, ty, &right, size, p.muted);
        // Bottom: a question, a message, or what the keys do here.
        let by = h - b + ty;
        if let Some(pr) = &self.prompt {
            let label = pr.label.clone();
            let text = pr.text.clone();
            let lw = self.ui.draw(&mut self.cv, 10, by, &label, size, p.mark());
            let tw = self
                .ui
                .draw(&mut self.cv, 10 + lw + 8, by, &text, size, p.foreground);
            if !matches!(
                pr.ask,
                crate::app::Ask::Overwrite | crate::app::Ask::Trash | crate::app::Ask::Unsaved(_)
            ) {
                self.cv.fill(
                    Rect::new(10 + lw + 8 + tw + 1, by, 2, size.height),
                    p.foreground,
                );
            }
            return;
        }
        let (text, color) = match &self.message {
            Some((m, _, err)) => (m.clone(), if *err { p.error } else { p.foreground }),
            None => (self.hints(), p.muted),
        };
        let text = self.ui.ellipsize(&text, size, w - 20);
        self.ui.draw(&mut self.cv, 10, by, &text, size, color);
    }

    fn hints(&self) -> String {
        if self.gallery {
            return "Arrows choose   Enter opens   Esc back   ? help".into();
        }
        match &self.doc {
            Doc::Pdf(d) if d.sel.is_some() => {
                "Drag moves   corner resizes   arrows nudge   Delete removes   Ctrl+S saves a signed copy".into()
            }
            Doc::Pdf(_) => "S sign   T text   D date   X cross   wheel scrolls   Ctrl+S save a signed copy   ? help".into(),
            Doc::Pic(_) if self.tool != Tool::None => {
                format!("{}   ink {}   line {} px   Esc done", self.tool.label(), INKS[self.ink].1, self.width)
            }
            Doc::Pic(_) => "Arrows next   G gallery   S slideshow   R rotate   C crop   P pen   X pixelate   ? help".into(),
            _ => "? help".into(),
        }
    }

    fn toast(&mut self, m: &str, err: bool) {
        let p = self.pal;
        let size = self.ui.size(self.font_px);
        let m = self.ui.ellipsize(m, size, self.cv.w - 80);
        let tw = self.ui.width(&m, size);
        let r = Rect::new(
            (self.cv.w - tw) / 2 - 12,
            self.cv.h - size.height - 40,
            tw + 24,
            size.height + 14,
        );
        self.cv.tint(r, p.background, 220);
        self.cv.frame(r, p.border);
        self.ui.draw(
            &mut self.cv,
            r.x + 12,
            r.y + 7,
            &m,
            size,
            if err { p.error } else { p.foreground },
        );
    }

    fn panel(&mut self, r: Rect) {
        let p = self.pal;
        self.cv.fill(
            Rect::new(r.x + 4, r.y + 4, r.w, r.h),
            theme::blend(p.background, 0x000000, 0.6),
        );
        self.cv.fill(r, p.background);
        self.cv.frame(r, p.border);
        self.cv.fill(Rect::new(r.x, r.y, r.w, 2), p.mark());
    }

    fn draw_info(&mut self) {
        let p = self.pal;
        let size = self.ui.size(self.font_px);
        let mut lines: Vec<(String, String)> = Vec::new();
        if let Some(path) = self.doc.path() {
            lines.push(("Name".into(), crate::app::name_of(path)));
            if let Some(dir) = path.parent() {
                let home = crate::files::home();
                let d = dir
                    .strip_prefix(&home)
                    .map(|r| format!("~/{}", r.display()))
                    .unwrap_or_else(|_| dir.display().to_string());
                lines.push(("Folder".into(), d));
            }
            if let Ok(m) = std::fs::metadata(path) {
                lines.push(("Size".into(), human(m.len())));
            }
        }
        match &self.doc {
            Doc::Pic(d) => {
                lines.push((
                    "Pixels".into(),
                    format!("{} x {}", d.image.width(), d.image.height()),
                ));
                if !d.frames.is_empty() {
                    lines.push(("Frames".into(), (d.frames.len() + 1).to_string()));
                }
                let f = &d.facts;
                for (k, v) in [
                    ("Taken", &f.taken),
                    ("Camera", &f.camera),
                    ("Exposure", &f.exposure),
                    ("Place", &f.place),
                ] {
                    if let Some(v) = v {
                        lines.push((k.into(), v.clone()));
                    }
                }
            }
            Doc::Pdf(d) => {
                lines.push(("Pages".into(), d.info.pages.len().to_string()));
                if !d.info.title.is_empty() {
                    lines.push(("Title".into(), d.info.title.clone()));
                }
                let (w, h) = d.info.pages[0].shown();
                lines.push((
                    "Page size".into(),
                    format!("{:.0} x {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4),
                ));
            }
            _ => {}
        }
        let kw = lines
            .iter()
            .map(|(k, _)| self.ui.width(k, size))
            .max()
            .unwrap_or(0);
        let vw = lines
            .iter()
            .map(|(_, v)| self.ui.width(v, size))
            .max()
            .unwrap_or(0)
            .min(self.cv.w / 2);
        let lh = size.height + 4;
        let r = Rect::new(
            self.cv.w - kw - vw - 60,
            self.bar_h() + 12,
            kw + vw + 44,
            lines.len() as i32 * lh + 20,
        );
        self.panel(r);
        for (n, (k, v)) in lines.iter().enumerate() {
            let y = r.y + 12 + n as i32 * lh;
            self.ui.draw(&mut self.cv, r.x + 14, y, k, size, p.muted);
            let v = self.ui.ellipsize(v, size, vw);
            self.ui
                .draw(&mut self.cv, r.x + 14 + kw + 16, y, &v, size, p.foreground);
        }
    }

    fn draw_help(&mut self) {
        let p = self.pal;
        let size = self.ui.size(self.font_px);
        let lh = size.height + 3;
        let kw = HELP
            .iter()
            .map(|(k, _)| self.ui.width(k, size))
            .max()
            .unwrap_or(0);
        let dw = HELP
            .iter()
            .map(|(_, d)| self.ui.width(d, size))
            .max()
            .unwrap_or(0);
        let w = (kw + dw + 60).min(self.cv.w - 20);
        let h = (HELP.len() as i32 * lh + 50).min(self.cv.h - 20);
        let r = Rect::new((self.cv.w - w) / 2, (self.cv.h - h) / 2, w, h);
        self.panel(r);
        self.ui.draw(
            &mut self.cv,
            r.x + 16,
            r.y + 12,
            "NIRUEDITVIEW",
            size,
            p.mark(),
        );
        for (n, (k, d)) in HELP.iter().enumerate() {
            let y = r.y + 36 + n as i32 * lh;
            if y + lh > r.y + r.h {
                break;
            }
            self.ui
                .draw(&mut self.cv, r.x + 16, y, k, size, p.foreground);
            self.ui
                .draw(&mut self.cv, r.x + 16 + kw + 24, y, d, size, p.muted);
        }
    }

    fn draw_pad(&mut self) {
        let p = self.pal;
        let size = self.ui.size(self.font_px);
        let (w, h) = (self.cv.w, self.cv.h);
        self.cv.tint(Rect::new(0, 0, w, h), 0x000000, 140);
        let r = self.pad_rect();
        let outer = Rect::new(
            r.x - 16,
            r.y - size.height - 24,
            r.w + 32,
            r.h + 2 * size.height + 48,
        );
        self.panel(outer);
        self.ui.draw(
            &mut self.cv,
            outer.x + 16,
            outer.y + 10,
            "Sign with the mouse or the touchpad",
            size,
            p.foreground,
        );
        self.cv.fill(r, 0xfbfaf6);
        self.cv.fill(
            Rect::new(r.x + 24, r.y + r.h * 3 / 4, r.w - 48, 1),
            0xb8b4aa,
        );
        if let Some(strokes) = &self.pad {
            let strokes = strokes.clone();
            for s in strokes {
                let pts: Vec<(f64, f64)> = s
                    .iter()
                    .map(|q| (r.x as f64 + q.0, r.y as f64 + q.1))
                    .collect();
                self.cv.stroke(&pts, 0x14183c, 2.6);
            }
        }
        let hint =
            "Enter uses it   Backspace clears   Esc cancels   or drop a picture of your signature";
        let hint = self.ui.ellipsize(hint, size, outer.w - 32);
        self.ui.draw(
            &mut self.cv,
            outer.x + 16,
            r.y + r.h + 12,
            &hint,
            size,
            p.muted,
        );
    }
}

fn rect_of(a: (f64, f64), b: (f64, f64)) -> Rect {
    let (x0, y0) = (a.0.min(b.0).round() as i32, a.1.min(b.1).round() as i32);
    let (x1, y1) = (a.0.max(b.0).round() as i32, a.1.max(b.1).round() as i32);
    Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

pub fn human(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.1} GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.0} kB", f / 1e3)
    } else {
        format!("{n} bytes")
    }
}
