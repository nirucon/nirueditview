//! The viewer: what is open, how it is shown, and what the keys and the
//! mouse do. Drawing is in `ui.rs`.
use crate::canvas::{Canvas, Rect};
use crate::files::{self, Sort};
use crate::jobs::{Done, Job, Jobs};
use crate::pdftools::{Find, TextSel};
use crate::text::Text;
use crate::x::{keys, X};
use crate::{forms, pdf, pdfpages, pdftext, picture, theme};
use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{ButtonPressEvent, KeyButMask};
use x11rb::protocol::Event;

pub struct PicDoc {
    pub path: PathBuf,
    pub image: RgbaImage,
    pub frames: Vec<(RgbaImage, u32)>,
    pub first_delay: u32,
    pub frame: usize,
    pub next_frame: Option<Instant>,
    pub facts: picture::Facts,
    pub undo: Vec<RgbaImage>,
    pub redo: Vec<RgbaImage>,
    pub modified: bool,
    /// Bumped on every change, so the view caches know to redo their work.
    pub generation: u64,
    /// GPS in XMP: looked for when the details are first shown.
    pub xmp_gps: Option<bool>,
}

impl PicDoc {
    /// The frame on screen (the picture itself unless it is animated).
    pub fn shown(&self) -> &RgbaImage {
        if self.frame == 0 || self.frames.is_empty() {
            &self.image
        } else {
            &self.frames[self.frame - 1].0
        }
    }

    fn change(&mut self, next: RgbaImage) {
        let old = std::mem::replace(&mut self.image, next);
        self.undo.push(old);
        if self.undo.len() > 12 {
            self.undo.remove(0);
        }
        self.redo.clear();
        // An edited animation is its first frame from now on.
        self.frames.clear();
        self.frame = 0;
        self.next_frame = None;
        self.modified = true;
        self.generation += 1;
    }
}

pub struct PageRaster {
    pub dpi: u32,
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

pub struct PdfDoc {
    pub path: PathBuf,
    pub info: pdf::Info,
    /// Each page's top, in points from the top of the first page.
    pub tops: Vec<f64>,
    pub total_h: f64,
    pub max_w: f64,
    pub pages: HashMap<usize, PageRaster>,
    pub stamps: Vec<pdf::Stamp>,
    pub sel: Option<usize>,
    /// The words, read when text is first searched or selected.
    pub words: Option<pdftext::Words>,
    pub words_asked: bool,
    /// The form, read when it is first filled in.
    pub fields: Option<Vec<forms::Field>>,
    /// The pages view: the new order, what to undo, its thumbnails.
    pub plan: Option<pdfpages::Plan>,
    pub plan_undo: Vec<pdfpages::Plan>,
    pub strip: HashMap<(PathBuf, usize), PageRaster>,
}

impl PdfDoc {
    pub const GAP: f64 = 14.0;

    fn new(path: PathBuf, info: pdf::Info) -> Self {
        let mut tops = Vec::new();
        let mut y = 0.0;
        let mut max_w: f64 = 1.0;
        for p in &info.pages {
            tops.push(y);
            let (w, h) = p.shown();
            y += h + Self::GAP;
            max_w = max_w.max(w);
        }
        Self {
            path,
            info,
            tops,
            total_h: (y - Self::GAP).max(1.0),
            max_w,
            pages: HashMap::new(),
            stamps: Vec::new(),
            sel: None,
            words: None,
            words_asked: false,
            fields: None,
            plan: None,
            plan_undo: Vec::new(),
            strip: HashMap::new(),
        }
    }

    pub fn fields_changed(&self) -> bool {
        self.fields
            .as_ref()
            .is_some_and(|f| f.iter().any(|f| f.changed))
    }

    pub fn plan_changed(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|p| !p.untouched(self.info.pages.len()))
    }

    pub fn modified(&self) -> bool {
        !self.stamps.is_empty() || self.fields_changed() || self.plan_changed()
    }

    /// Forget what was added, filled in and moved.
    pub fn discard(&mut self) {
        self.stamps.clear();
        self.sel = None;
        self.fields = None;
        self.plan = None;
        self.plan_undo.clear();
    }
}

pub enum Doc {
    Empty(String),
    Loading(PathBuf),
    Pic(PicDoc),
    Pdf(PdfDoc),
}

impl Doc {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Doc::Empty(_) => None,
            Doc::Loading(p) => Some(p),
            Doc::Pic(d) => Some(&d.path),
            Doc::Pdf(d) => Some(&d.path),
        }
    }
    fn modified(&self) -> bool {
        match self {
            Doc::Pic(d) => d.modified,
            Doc::Pdf(d) => d.modified(),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    None,
    Crop,
    Pen,
    Arrow,
    Box,
    Text,
    Blur,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::None => "",
            Tool::Crop => "crop: drag, Enter crops",
            Tool::Pen => "pen: draw",
            Tool::Arrow => "arrow: drag",
            Tool::Box => "box: drag",
            Tool::Text => "text: click where it goes",
            Tool::Blur => "pixelate: drag over it",
        }
    }
}

pub enum Drag {
    None,
    /// A click that may become a pan.
    Press {
        x: i32,
        y: i32,
        ox: f64,
        oy: f64,
        scroll: f64,
    },
    Pan {
        x: i32,
        y: i32,
        ox: f64,
        oy: f64,
        scroll: f64,
    },
    Line(Vec<(f64, f64)>),
    Rect((f64, f64), (f64, f64)),
    Move {
        idx: usize,
        du: f64,
        dv: f64,
    },
    Resize {
        idx: usize,
    },
    Pad,
    /// Choosing text on a PDF (Shift and a drag); the ends are in `text_sel`.
    Select,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum After {
    Open(usize),
    Quit,
    LeavePages,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Resize,
    PicText {
        at: (f64, f64),
    },
    PdfText,
    Overwrite,
    Trash,
    Unsaved(After),
    /// Fill in form field `n`.
    Field(usize),
    /// A format and quality to save a picture as.
    SaveAs,
    /// A PDF whose pages go at the end.
    Merge,
    /// Pages saved as a file of their own.
    Extract,
}

/// A picture edit shown before it is made: brightness and contrast, or
/// straightening by a small angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adjust {
    Light { brightness: i32, contrast: i32 },
    Straighten { deg: f64 },
}

impl Adjust {
    fn clamp(self) -> Self {
        match self {
            Adjust::Straighten { deg } => Adjust::Straighten {
                deg: deg.clamp(-20.0, 20.0),
            },
            a => a,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Adjust::Light {
                brightness,
                contrast,
            } => format!(
                "Brightness {brightness:+}   contrast {contrast:+}   (Up Down, Left Right, Shift finer, Enter applies)"
            ),
            Adjust::Straighten { deg } => format!(
                "Straighten {deg:+.1}°   (Left Right, Shift finer, Enter applies, Esc cancels)"
            ),
        }
    }
}

fn adjust_hint(a: Adjust) -> &'static str {
    match a {
        Adjust::Light { .. } => {
            "Up and Down: brightness, Left and Right: contrast, Shift finer, 0 resets, Enter applies, Esc cancels"
        }
        Adjust::Straighten { .. } => {
            "Left and Right turn the picture (Shift: finer), line it up with the grid, Enter cuts it straight"
        }
    }
}

pub struct Prompt {
    pub ask: Ask,
    pub label: String,
    pub text: String,
}

pub struct Slides {
    pub every: Duration,
    pub next: Instant,
    pub paused: bool,
    pub was_fullscreen: bool,
}

/// A stamp following the pointer until a click puts it down.
pub struct Placing {
    pub stamp: pdf::Stamp,
}

pub const INKS: [(u32, &str); 5] = [
    (0xe03c31, "red"),
    (0x111111, "black"),
    (0xffffff, "white"),
    (0xf2c12e, "yellow"),
    (0x2f6fdf, "blue"),
];

pub struct App {
    pub x: X,
    pub cv: Canvas,
    pub ui: Text,
    pub sans: Text,
    pub pal: theme::Palette,
    pub font_px: f32,
    pub jobs: Jobs,
    pub list: Vec<PathBuf>,
    pub index: usize,
    pub sort: Sort,
    pub doc: Doc,
    /// Decoded neighbours, ready when the page turns.
    pub ready: HashMap<PathBuf, picture::Picture>,
    pub zoom: f64,
    pub fit: bool,
    pub ox: f64,
    pub oy: f64,
    pub scroll: f64,
    pub scaled: Option<(u64, u64, u32, u32, Vec<u32>)>,
    pub full: Option<(u64, Vec<u32>)>,
    pub tool: Tool,
    pub ink: usize,
    pub width: f64,
    pub drag: Drag,
    pub crop: Option<((f64, f64), (f64, f64))>,
    pub prompt: Option<Prompt>,
    pub placing: Option<Placing>,
    pub pad: Option<Vec<Vec<(f64, f64)>>>,
    pub help: bool,
    pub info: bool,
    pub gallery: bool,
    pub gallery_scroll: i32,
    pub thumbs: HashMap<PathBuf, Option<(u32, u32, Vec<u32>)>>,
    pub slides: Option<Slides>,
    pub message: Option<(String, Instant, bool)>,
    pub mouse: (i32, i32),
    pub last_click: Option<(Instant, i32, i32)>,
    pub dirty: bool,
    pub quit: bool,
    pub clip_png: Vec<u8>,
    pub clip_text: String,
    pub find: Option<Find>,
    pub text_sel: Option<TextSel>,
    /// Filling in the PDF's form (E).
    pub form: bool,
    pub field_sel: Option<usize>,
    /// The pages view of a PDF (P).
    pub pages_view: bool,
    pub page_sel: usize,
    pub strip_scroll: i32,
    pub adjust: Option<Adjust>,
}

impl App {
    pub fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        let x = X::open("NIRUEDITVIEW").map_err(|e| format!("no X display: {e}"))?;
        let base = theme::config_home();
        let name = x
            .root_theme()
            .unwrap_or_else(|| theme::configured_theme(&base));
        let pal = theme::load(&base, &name);
        let font_px = theme::configured_font_size(&base).unwrap_or(13.0);
        let ui = Text::new(theme::configured_font_path(&base).as_deref())?;
        let sans = Text::new_sans().or_else(|_| Text::new(None))?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 4))
            .unwrap_or(2);
        let mut app = Self {
            x,
            cv: Canvas::new(800, 600),
            ui,
            sans,
            pal,
            font_px,
            jobs: Jobs::start(threads),
            list: Vec::new(),
            index: 0,
            sort: Sort::Name,
            doc: Doc::Empty("Drop a picture or a PDF here, or open one: nirueditview FILE".into()),
            ready: HashMap::new(),
            zoom: 1.0,
            fit: true,
            ox: 0.0,
            oy: 0.0,
            scroll: 0.0,
            scaled: None,
            full: None,
            tool: Tool::None,
            ink: 0,
            width: 4.0,
            drag: Drag::None,
            crop: None,
            prompt: None,
            placing: None,
            pad: None,
            help: false,
            info: false,
            gallery: false,
            gallery_scroll: 0,
            thumbs: HashMap::new(),
            slides: None,
            message: None,
            mouse: (0, 0),
            last_click: None,
            dirty: true,
            quit: false,
            clip_png: Vec::new(),
            clip_text: String::new(),
            find: None,
            text_sel: None,
            form: false,
            field_sel: None,
            pages_view: false,
            page_sel: 0,
            strip_scroll: 0,
            adjust: None,
        };
        app.open_paths(paths);
        Ok(app)
    }

    // ------------------------------------------------------------ files

    /// One file: its folder becomes the list. Several: just those. A folder:
    /// its pictures, in the gallery.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter_map(|p| std::fs::canonicalize(&p).ok())
            .collect();
        if paths.is_empty() {
            return;
        }
        if paths.len() == 1 {
            let p = &paths[0];
            if p.is_dir() {
                self.list = files::list(p, self.sort);
                self.index = 0;
                if self.list.is_empty() {
                    self.doc = Doc::Empty(format!("No pictures or PDFs in {}", p.display()));
                    self.dirty = true;
                    return;
                }
                self.gallery = true;
                self.load_current();
                return;
            }
            let dir = p
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(files::home);
            self.list = files::list(&dir, self.sort);
            if !self.list.contains(p) {
                self.list.insert(0, p.clone());
            }
            self.index = self.list.iter().position(|q| q == p).unwrap_or(0);
        } else {
            self.list = paths.into_iter().filter(|p| files::shown(p)).collect();
            self.index = 0;
        }
        self.load_current();
    }

    pub fn current(&self) -> Option<&PathBuf> {
        self.list.get(self.index)
    }

    pub fn load_current(&mut self) {
        let Some(path) = self.current().cloned() else {
            self.doc = Doc::Empty("Nothing to show".into());
            self.dirty = true;
            return;
        };
        self.tool = Tool::None;
        self.crop = None;
        self.placing = None;
        self.drag = Drag::None;
        self.find = None;
        self.text_sel = None;
        self.form = false;
        self.field_sel = None;
        self.pages_view = false;
        self.adjust = None;
        self.scaled = None;
        self.full = None;
        self.fit = true;
        self.scroll = 0.0;
        if picture::is_pdf(&path) {
            match pdf::info(&path) {
                Ok(info) if info.encrypted => {
                    self.doc = Doc::Empty(format!(
                        "{} is encrypted (password protected): it cannot be shown here",
                        name_of(&path)
                    ))
                }
                Ok(info) => self.doc = Doc::Pdf(PdfDoc::new(path.clone(), info)),
                Err(e) => self.doc = Doc::Empty(format!("{}: {e}", name_of(&path))),
            }
        } else if let Some(pic) = self.ready.remove(&path) {
            self.doc = Doc::Pic(pic_doc(path.clone(), pic));
        } else {
            self.doc = Doc::Loading(path.clone());
            self.jobs.ask(Job::Picture {
                path: path.clone(),
                preload: false,
            });
        }
        self.x
            .set_title(&format!("{} · NIRUEDITVIEW", name_of(&path)));
        self.preload();
        self.dirty = true;
    }

    /// Decode the next and previous pictures now, and forget the rest.
    fn preload(&mut self) {
        let n = self.list.len();
        if n < 2 {
            return;
        }
        let near: Vec<PathBuf> = [1, n - 1, 2]
            .iter()
            .map(|d| self.list[(self.index + d) % n].clone())
            .filter(|p| picture::is_picture(p))
            .collect();
        self.ready.retain(|p, _| near.contains(p));
        self.jobs
            .drop_where(|j| matches!(j, Job::Picture { preload: true, .. }));
        for p in near {
            if !self.ready.contains_key(&p) {
                self.jobs.ask(Job::Picture {
                    path: p,
                    preload: true,
                });
            }
        }
    }

    pub fn go(&mut self, to: usize) {
        if self.list.is_empty() {
            return;
        }
        let to = to.min(self.list.len() - 1);
        if self.doc.modified() {
            self.ask(
                Ask::Unsaved(After::Open(to)),
                "Unsaved changes: S saves a copy, D discards, Esc stays",
            );
            return;
        }
        self.index = to;
        self.load_current();
    }

    pub fn step(&mut self, d: i64) {
        let n = self.list.len() as i64;
        if n == 0 {
            return;
        }
        self.go(((self.index as i64 + d).rem_euclid(n)) as usize);
    }

    pub fn resort(&mut self) {
        self.sort = self.sort.next();
        let cur = self.current().cloned();
        if let Some(dir) = cur.as_ref().and_then(|p| p.parent()) {
            self.list = files::list(dir, self.sort);
        }
        if let Some(c) = cur {
            self.index = self.list.iter().position(|p| *p == c).unwrap_or(0);
        }
        self.flash(&format!("Sorted by {}", self.sort.label()), false);
        self.preload();
    }

    pub fn flash(&mut self, msg: &str, error: bool) {
        self.message = Some((msg.to_string(), Instant::now(), error));
        self.dirty = true;
    }

    pub fn ask(&mut self, ask: Ask, label: &str) {
        let text = match &ask {
            Ask::Resize => "50%".to_string(),
            _ => String::new(),
        };
        self.ask_with(ask, label, text);
    }

    /// A question with an answer already written, to change or keep.
    pub fn ask_with(&mut self, ask: Ask, label: &str, text: String) {
        self.prompt = Some(Prompt {
            ask,
            label: label.to_string(),
            text,
        });
        self.dirty = true;
    }

    // ------------------------------------------------------------ geometry

    pub fn bar_h(&self) -> i32 {
        if self.x.fullscreen || self.slides.is_some() {
            0
        } else {
            self.ui.size(self.font_px).height + 10
        }
    }

    pub fn view(&self) -> Rect {
        let b = self.bar_h();
        Rect::new(0, b, self.cv.w, (self.cv.h - 2 * b).max(1))
    }

    /// Fit the picture or the page width, and centre.
    pub fn refit(&mut self) {
        let v = self.view();
        match &self.doc {
            Doc::Pic(d) => {
                let (iw, ih) = (d.image.width() as f64, d.image.height() as f64);
                if self.fit {
                    self.zoom = (v.w as f64 / iw).min(v.h as f64 / ih).min(1.0);
                    if self.slides.is_some() || self.x.fullscreen {
                        self.zoom = (v.w as f64 / iw).min(v.h as f64 / ih);
                    }
                }
                self.clamp_pan();
            }
            Doc::Pdf(d) => {
                if self.fit {
                    self.zoom = ((v.w as f64 - 40.0) / d.max_w).clamp(0.1, 8.0);
                }
                self.clamp_pan();
            }
            _ => {}
        }
    }

    pub fn clamp_pan(&mut self) {
        let v = self.view();
        match &self.doc {
            Doc::Pic(d) => {
                let (w, h) = (
                    d.image.width() as f64 * self.zoom,
                    d.image.height() as f64 * self.zoom,
                );
                let place = |o: f64, size: f64, vx: i32, vw: i32| {
                    if size <= vw as f64 {
                        vx as f64 + (vw as f64 - size) / 2.0
                    } else {
                        o.clamp(vx as f64 + vw as f64 - size, vx as f64)
                    }
                };
                if self.fit {
                    self.ox = v.x as f64 + (v.w as f64 - w) / 2.0;
                    self.oy = v.y as f64 + (v.h as f64 - h) / 2.0;
                } else {
                    self.ox = place(self.ox, w, v.x, v.w);
                    self.oy = place(self.oy, h, v.y, v.h);
                }
            }
            Doc::Pdf(d) => {
                let content = d.total_h * self.zoom + 40.0;
                self.scroll = self.scroll.clamp(0.0, (content - v.h as f64).max(0.0));
                let w = d.max_w * self.zoom;
                if w + 40.0 <= v.w as f64 {
                    self.ox = (v.w as f64 - w) / 2.0;
                } else {
                    self.ox = self.ox.clamp(v.w as f64 - w - 20.0, 20.0);
                }
            }
            _ => {}
        }
    }

    pub fn zoom_at(&mut self, factor: f64, at: (i32, i32)) {
        let old = self.zoom;
        let new = (old * factor).clamp(0.02, 32.0);
        self.fit = false;
        match &self.doc {
            Doc::Pic(_) => {
                let (ix, iy) = ((at.0 as f64 - self.ox) / old, (at.1 as f64 - self.oy) / old);
                self.zoom = new;
                self.ox = at.0 as f64 - ix * new;
                self.oy = at.1 as f64 - iy * new;
            }
            Doc::Pdf(_) => {
                let v = self.view();
                let doc_y = (self.scroll + (at.1 - v.y) as f64 - 20.0) / old;
                let doc_x = (at.0 as f64 - self.ox) / old;
                self.zoom = new.clamp(0.1, 8.0);
                self.scroll = doc_y * self.zoom + 20.0 - (at.1 - v.y) as f64;
                self.ox = at.0 as f64 - doc_x * self.zoom;
            }
            _ => {}
        }
        self.clamp_pan();
        self.dirty = true;
    }

    pub fn set_zoom(&mut self, z: f64) {
        let v = self.view();
        let c = (v.x + v.w / 2, v.y + v.h / 2);
        let f = z / self.zoom;
        self.zoom_at(f, c);
    }

    /// Screen to picture pixels.
    pub fn to_image(&self, x: i32, y: i32) -> (f64, f64) {
        (
            (x as f64 - self.ox) / self.zoom,
            (y as f64 - self.oy) / self.zoom,
        )
    }

    pub fn to_screen(&self, p: (f64, f64)) -> (f64, f64) {
        (self.ox + p.0 * self.zoom, self.oy + p.1 * self.zoom)
    }

    /// Where page `i` is on screen (its shown size times the zoom).
    pub fn page_rect(&self, d: &PdfDoc, i: usize) -> (f64, f64, f64, f64) {
        let v = self.view();
        let (w, h) = d.info.pages[i].shown();
        let x = self.ox + (d.max_w - w) * self.zoom / 2.0;
        let y = v.y as f64 + 20.0 + d.tops[i] * self.zoom - self.scroll;
        (x, y, w * self.zoom, h * self.zoom)
    }

    /// The page under a screen point and the point on it (shown points).
    pub fn page_at(&self, d: &PdfDoc, x: i32, y: i32) -> Option<(usize, f64, f64)> {
        (0..d.info.pages.len()).find_map(|i| {
            let (px, py, pw, ph) = self.page_rect(d, i);
            let (fx, fy) = (x as f64, y as f64);
            (fx >= px && fy >= py && fx < px + pw && fy < py + ph)
                .then(|| (i, (fx - px) / self.zoom, (fy - py) / self.zoom))
        })
    }

    pub fn current_page(&self, d: &PdfDoc) -> usize {
        let v = self.view();
        let mid = (self.scroll + v.h as f64 / 3.0 - 20.0) / self.zoom;
        d.tops.iter().rposition(|t| *t <= mid).unwrap_or(0)
    }

    pub fn scroll_to_page(&mut self, i: usize) {
        if let Doc::Pdf(d) = &self.doc {
            let i = i.min(d.tops.len().saturating_sub(1));
            self.scroll = d.tops[i] * self.zoom;
            self.clamp_pan();
            self.dirty = true;
        }
    }

    /// The DPI a page is drawn at for this zoom, in steps so a small zoom
    /// change does not redraw every page.
    pub fn dpi(&self) -> u32 {
        let want = 72.0 * self.zoom;
        let step = 12.0;
        ((want / step).ceil() * step).clamp(24.0, 600.0) as u32
    }

    // ------------------------------------------------------------ results

    fn job_done(&mut self, done: Done) {
        match done {
            Done::Picture(path, r) => {
                let wanted = matches!(&self.doc, Doc::Loading(p) if *p == path);
                match r {
                    Ok(pic) if wanted => {
                        self.doc = Doc::Pic(pic_doc(path, pic));
                        self.refit();
                    }
                    Ok(pic) => {
                        let n = self.list.len();
                        let near = (1..=2)
                            .chain([n.saturating_sub(1)])
                            .any(|d| n > 0 && self.list[(self.index + d) % n] == path);
                        if near {
                            self.ready.insert(path, pic);
                        }
                    }
                    Err(e) if wanted => {
                        self.doc = Doc::Empty(format!("{}: {e}", name_of(&path)));
                    }
                    Err(_) => {}
                }
            }
            Done::Page(path, page, dpi, Ok(raster)) if dpi == crate::pdftools::STRIP_DPI => {
                let px = raster
                    .rgb
                    .chunks_exact(3)
                    .map(|c| {
                        0xff00_0000 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
                    })
                    .collect();
                let r = PageRaster {
                    dpi,
                    w: raster.w,
                    h: raster.h,
                    px,
                };
                self.strip_done(path, page, r);
            }
            Done::Words(path, r) => self.words_done(path, r),
            Done::Wallpaper(path, r) => match r {
                Ok(()) => self.flash(&format!("{} is the wallpaper now", name_of(&path)), false),
                Err(e) => self.flash(&format!("Wallpaper not set: {e}"), true),
            },
            Done::Page(path, page, dpi, r) => {
                if let Doc::Pdf(d) = &mut self.doc {
                    if d.path == path {
                        match r {
                            Ok(raster) => {
                                let px = raster
                                    .rgb
                                    .chunks_exact(3)
                                    .map(|c| {
                                        0xff00_0000
                                            | ((c[0] as u32) << 16)
                                            | ((c[1] as u32) << 8)
                                            | c[2] as u32
                                    })
                                    .collect();
                                d.pages.insert(
                                    page,
                                    PageRaster {
                                        dpi,
                                        w: raster.w,
                                        h: raster.h,
                                        px,
                                    },
                                );
                            }
                            Err(e) => self.flash(&format!("page {}: {e}", page + 1), true),
                        }
                    }
                }
            }
            Done::Thumb(path, img) => {
                // Fitted to the gallery's cell once, not on every frame.
                let cell = self.gallery_cell().0 as u32;
                let t = img.map(|i| {
                    let f = (cell as f64 / i.width().max(i.height()) as f64).min(1.0);
                    let (w, h) = (
                        ((i.width() as f64 * f).round() as u32).max(1),
                        ((i.height() as f64 * f).round() as u32).max(1),
                    );
                    if (w, h) == i.dimensions() {
                        (w, h, picture::argb(&i))
                    } else {
                        (w, h, picture::reduce(&i, w, h))
                    }
                });
                self.thumbs.insert(path, t);
            }
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------ editing

    fn ink_rgba(&self) -> [u8; 4] {
        let c = INKS[self.ink].0;
        [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
    }

    fn edit(&mut self, f: impl FnOnce(&RgbaImage) -> Option<RgbaImage>) {
        if let Doc::Pic(d) = &mut self.doc {
            if let Some(next) = f(&d.image) {
                d.change(next);
                self.scaled = None;
                self.full = None;
            }
        }
        self.refit();
        self.dirty = true;
    }

    fn undo(&mut self, redo: bool) {
        if let Doc::Pic(d) = &mut self.doc {
            let (from, to) = if redo {
                (&mut d.redo, &mut d.undo)
            } else {
                (&mut d.undo, &mut d.redo)
            };
            if let Some(img) = from.pop() {
                to.push(std::mem::replace(&mut d.image, img));
                d.generation += 1;
                d.modified = !d.undo.is_empty();
                self.scaled = None;
                self.full = None;
                self.refit();
            } else {
                let m = if redo {
                    "Nothing to redo"
                } else {
                    "Nothing to undo"
                };
                self.flash(m, false);
            }
        }
        self.dirty = true;
    }

    fn finish_drag(&mut self) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        let ink = self.ink_rgba();
        let width = self.width / self.zoom.max(0.01);
        match drag {
            Drag::Line(points) if self.tool == Tool::Pen => {
                self.edit(|img| {
                    let mut n = img.clone();
                    picture::stroke(&mut n, &points, ink, width);
                    Some(n)
                });
            }
            Drag::Rect(a, b) => match self.tool {
                Tool::Crop => {
                    self.crop = Some((a, b));
                    self.flash("Enter crops to the frame, Esc cancels", false);
                }
                Tool::Arrow => {
                    if (a.0 - b.0).abs() + (a.1 - b.1).abs() > 3.0 {
                        self.edit(|img| {
                            let mut n = img.clone();
                            for line in picture::arrow_lines(a, b, width) {
                                picture::stroke(&mut n, &line, ink, width);
                            }
                            Some(n)
                        });
                    }
                }
                Tool::Box => {
                    self.edit(|img| {
                        picture::clamp_rect(img, a, b)?;
                        let mut n = img.clone();
                        picture::stroke(&mut n, &picture::rect_lines(a, b), ink, width);
                        Some(n)
                    });
                }
                Tool::Blur => {
                    self.edit(|img| {
                        let r = picture::clamp_rect(img, a, b)?;
                        let mut n = img.clone();
                        let block = (img.width().max(img.height()) / 80).max(8);
                        picture::pixelate(&mut n, r, block);
                        Some(n)
                    });
                }
                _ => {}
            },
            Drag::Move { .. } | Drag::Resize { .. } => self.dirty = true,
            Drag::Press { x, y, .. } => self.click(x, y),
            _ => {}
        }
        self.dirty = true;
    }

    /// A click that did not move: in a picture, the left and right edges turn
    /// the page; a double click goes full screen.
    fn click(&mut self, x: i32, y: i32) {
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, lx, ly)| {
            now.duration_since(t) < Duration::from_millis(400)
                && (lx - x).abs() < 6
                && (ly - y).abs() < 6
        });
        self.last_click = Some((now, x, y));
        if double {
            self.last_click = None;
            self.x.toggle_fullscreen();
            return;
        }
        if matches!(self.doc, Doc::Pic(_)) && self.tool == Tool::None {
            let v = self.view();
            if x < v.x + v.w / 7 {
                self.step(-1);
            } else if x > v.x + v.w * 6 / 7 {
                self.step(1);
            }
        }
    }

    // ------------------------------------------------------------ saving

    pub fn save_copy(&mut self) -> bool {
        match &mut self.doc {
            Doc::Pic(d) => {
                let to = picture::edited_path(&d.path);
                match picture::save(&d.image, &to) {
                    Ok(()) => {
                        let name = name_of(&to);
                        d.path = to.clone();
                        d.modified = false;
                        d.undo.clear();
                        d.redo.clear();
                        self.after_save(to);
                        self.flash(
                            &format!("Saved as {name}; the original is unchanged"),
                            false,
                        );
                        true
                    }
                    Err(e) => {
                        self.flash(&format!("Not saved: {e}"), true);
                        false
                    }
                }
            }
            Doc::Pdf(d) if d.plan_changed() => self.save_pages(),
            Doc::Pdf(d) => {
                if d.stamps.is_empty() && !d.fields_changed() {
                    let m = if self.pages_view {
                        "No page changes yet: R turns, Delete removes, Shift+arrows move"
                    } else {
                        "Nothing added yet: S puts your signature, T text, D the date"
                    };
                    self.flash(m, false);
                    return false;
                }
                let suffix = if d.stamps.is_empty() {
                    "filled"
                } else {
                    "signed"
                };
                let to = pdf::copy_path(&d.path, suffix);
                let fields = d.fields.clone().unwrap_or_default();
                match pdf::write_stamps(&d.path, &to, &d.info.pages, &d.stamps, &fields) {
                    Ok(()) => {
                        let name = name_of(&to);
                        d.discard();
                        self.after_save(to);
                        self.load_current();
                        self.flash(
                            &format!("Saved as {name}; the original is unchanged"),
                            false,
                        );
                        true
                    }
                    Err(e) => {
                        self.flash(&format!("Not saved: {e}"), true);
                        false
                    }
                }
            }
            _ => false,
        }
    }

    /// Save the picture (as it is now) in another format: `jpg 85`, `png`
    /// or `webp`. A copy; the original stays.
    fn save_as(&mut self, text: &str) {
        let Some((ext, q)) = picture::parse_save_as(text) else {
            self.flash(
                "Write jpg, png or webp, and for jpg a quality: jpg 85",
                true,
            );
            return;
        };
        let Doc::Pic(d) = &mut self.doc else { return };
        let to = picture::converted_path(&d.path, ext);
        match picture::save_with(&d.image, &to, q) {
            Ok(()) => {
                let name = name_of(&to);
                d.path = to.clone();
                d.modified = false;
                d.undo.clear();
                d.redo.clear();
                self.after_save(to);
                let how = match ext {
                    "jpg" => format!(" at quality {q}"),
                    "webp" => " (lossless)".into(),
                    _ => String::new(),
                };
                self.flash(
                    &format!("Saved as {name}{how}; the original is unchanged"),
                    false,
                );
            }
            Err(e) => self.flash(&format!("Not saved: {e}"), true),
        }
    }

    /// A copy without where it was taken and without the camera details;
    /// the picture is the same (and stays upright).
    fn save_private(&mut self) {
        let Doc::Pic(d) = &self.doc else { return };
        let to = pdf::copy_path(&d.path, "private");
        let r = if d.modified {
            // Edited pixels are written anew, and nothing of the EXIF goes along.
            picture::save(&d.image, &to)
        } else if matches!(picture::ext(&d.path).as_deref(), Some("gif" | "bmp")) {
            self.flash(
                "A GIF or BMP picture has no camera details to remove",
                false,
            );
            return;
        } else {
            picture::save_private(&d.path, &d.image, &to)
        };
        match r {
            Ok(()) => {
                let name = name_of(&to);
                self.after_save(to);
                if let Doc::Pic(d) = &mut self.doc {
                    d.modified = false;
                }
                self.load_current();
                self.flash(
                    &format!("Saved as {name}, without location and camera details"),
                    false,
                );
            }
            Err(e) => self.flash(&format!("Not saved: {e}"), true),
        }
    }

    fn set_wallpaper(&mut self) {
        let Doc::Pic(d) = &self.doc else { return };
        if d.modified {
            self.flash(
                "Save the picture first (Ctrl+S), then make it the wallpaper",
                false,
            );
            return;
        }
        let path = d.path.clone();
        self.jobs.ask(Job::Wallpaper { path });
        self.flash("Setting the wallpaper…", false);
    }

    fn start_adjust(&mut self, a: Adjust) {
        if !matches!(self.doc, Doc::Pic(_)) {
            return;
        }
        self.tool = Tool::None;
        self.crop = None;
        if let Doc::Pic(d) = &mut self.doc {
            d.next_frame = None;
        }
        self.adjust = Some(a);
        self.flash(adjust_hint(a), false);
    }

    /// Keys while adjusting: arrows change it, Enter makes it, Esc leaves it.
    fn adjust_key(&mut self, sym: u32, shift: bool) {
        let Some(a) = self.adjust else { return };
        let step = if shift { 1 } else { 5 };
        let next = match (a, sym) {
            (_, keys::ESCAPE) => {
                self.adjust = None;
                return;
            }
            (_, keys::RETURN | keys::KP_ENTER) => {
                self.adjust = None;
                match a {
                    Adjust::Light {
                        brightness: 0,
                        contrast: 0,
                    } => {}
                    Adjust::Light {
                        brightness,
                        contrast,
                    } => self.edit(|i| Some(picture::light(i, brightness, contrast))),
                    Adjust::Straighten { deg } if deg.abs() < 0.01 => {}
                    Adjust::Straighten { deg } => {
                        self.edit(|i| Some(picture::straighten(i, deg)));
                        self.fit = true;
                        self.refit();
                    }
                }
                return;
            }
            (Adjust::Light { .. }, keys::BACKSPACE | 0x30) => Adjust::Light {
                brightness: 0,
                contrast: 0,
            },
            (
                Adjust::Light {
                    brightness,
                    contrast,
                },
                keys::UP | keys::DOWN | keys::LEFT | keys::RIGHT,
            ) => {
                let (db, dc) = match sym {
                    keys::UP => (step, 0),
                    keys::DOWN => (-step, 0),
                    keys::RIGHT => (0, step),
                    _ => (0, -step),
                };
                Adjust::Light {
                    brightness: (brightness + db).clamp(-100, 100),
                    contrast: (contrast + dc).clamp(-100, 100),
                }
            }
            (Adjust::Straighten { .. }, keys::BACKSPACE | 0x30) => Adjust::Straighten { deg: 0.0 },
            (Adjust::Straighten { deg }, keys::LEFT | keys::RIGHT) => {
                let d = if shift { 0.1 } else { 0.5 };
                let d = if sym == keys::RIGHT { d } else { -d };
                Adjust::Straighten {
                    deg: ((deg + d) * 10.0).round() / 10.0,
                }
                .clamp()
            }
            _ => return,
        };
        self.adjust = Some(next);
        self.flash(&next.label(), false);
    }

    pub fn after_save(&mut self, to: PathBuf) {
        if let Some(dir) = to.parent() {
            self.list = files::list(dir, self.sort);
        }
        self.index = self.list.iter().position(|p| *p == to).unwrap_or(0);
        self.x
            .set_title(&format!("{} · NIRUEDITVIEW", name_of(&to)));
        self.thumbs.remove(&to);
    }

    fn overwrite(&mut self) {
        if let Doc::Pic(d) = &mut self.doc {
            match picture::save(&d.image, &d.path) {
                Ok(()) => {
                    d.modified = false;
                    d.undo.clear();
                    d.redo.clear();
                    let p = d.path.clone();
                    self.thumbs.remove(&p);
                    self.flash(&format!("Saved {}", name_of(&p)), false);
                }
                Err(e) => self.flash(&format!("Not saved: {e}"), true),
            }
        }
    }

    fn trash_current(&mut self) {
        let Some(p) = self.current().cloned() else {
            return;
        };
        match files::trash(&p) {
            Ok(()) => {
                self.flash(&format!("{} is in the trash", name_of(&p)), false);
                self.list.remove(self.index);
                self.thumbs.remove(&p);
                if self.index >= self.list.len() {
                    self.index = self.list.len().saturating_sub(1);
                }
                if let Doc::Pic(d) = &mut self.doc {
                    d.modified = false;
                }
                self.load_current();
            }
            Err(e) => self.flash(&format!("Not moved to the trash: {e}"), true),
        }
    }

    pub fn copy_to_clipboard(&mut self) {
        if matches!(self.doc, Doc::Pdf(_)) {
            self.copy_text();
            return;
        }
        if let Doc::Pic(d) = &self.doc {
            self.clip_text.clear();
            self.clip_png = picture::png_bytes(d.shown());
            self.x.own_clipboard();
            self.flash("The picture is on the clipboard", false);
        }
    }

    // ------------------------------------------------------------ PDF

    fn signature_path() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| files::home().join(".local/share"))
            .join("nirueditview/signature.png")
    }

    fn start_signature(&mut self, fresh: bool) {
        if !matches!(self.doc, Doc::Pdf(_)) {
            return;
        }
        if !fresh {
            if let Ok(img) = image::open(Self::signature_path()) {
                self.place_picture(img.into_rgba8());
                return;
            }
        }
        self.pad = Some(Vec::new());
        self.dirty = true;
    }

    fn place_picture(&mut self, img: RgbaImage) {
        let (w, h) = img.dimensions();
        let pw = 160.0;
        let ph = pw * h as f64 / w.max(1) as f64;
        self.placing = Some(Placing {
            stamp: pdf::Stamp {
                page: 0,
                u: 0.0,
                v: 0.0,
                w: pw,
                h: ph,
                kind: pdf::Kind::Picture {
                    w,
                    h,
                    rgba: Arc::new(img.into_raw()),
                },
                color: [0.0; 3],
            },
        });
        self.flash("Click where it goes; drag the corner to resize", false);
        self.follow(self.mouse.0, self.mouse.1);
    }

    fn place_text(&mut self, text: String, size: f64) {
        let w = pdf::helvetica_width(&text, size);
        // Text on a document is black, or blue ink when that is chosen.
        let color = if INKS[self.ink].1 == "blue" {
            [0.18, 0.44, 0.87]
        } else {
            [0.0, 0.0, 0.0]
        };
        self.placing = Some(Placing {
            stamp: pdf::Stamp {
                page: 0,
                u: 0.0,
                v: 0.0,
                w,
                h: size * pdf::TEXT_HEIGHT,
                kind: pdf::Kind::Text { text, size },
                color,
            },
        });
        self.flash("Click where it goes", false);
        self.follow(self.mouse.0, self.mouse.1);
    }

    /// Move the stamp being placed under the pointer.
    fn follow(&mut self, x: i32, y: i32) {
        let Doc::Pdf(d) = &self.doc else { return };
        let hit = self.page_at(d, x, y);
        if let (Some(pl), Some((page, u, v))) = (self.placing.as_mut(), hit) {
            pl.stamp.page = page;
            pl.stamp.u = u - pl.stamp.w / 2.0;
            pl.stamp.v = v - pl.stamp.h / 2.0;
        }
        self.dirty = true;
    }

    /// The signature pad, drawn into a picture: dark ink, trimmed to the
    /// strokes, transparent around them.
    pub fn pad_rect(&self) -> Rect {
        let (w, h) = (640.min(self.cv.w - 40), 240.min(self.cv.h - 120));
        Rect::new((self.cv.w - w) / 2, (self.cv.h - h) / 2, w, h)
    }

    fn finish_pad(&mut self) {
        let Some(strokes) = self.pad.take() else {
            return;
        };
        if strokes.iter().all(|s| s.is_empty()) {
            self.flash("Sign in the box first", false);
            self.pad = Some(strokes);
            return;
        }
        let scale = 3.0;
        let pts: Vec<(f64, f64)> = strokes.iter().flatten().copied().collect();
        let (minx, maxx) = pts
            .iter()
            .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (miny, maxy) = pts
            .iter()
            .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let pad = 6.0;
        let w = ((maxx - minx + 2.0 * pad) * scale).ceil().max(4.0) as u32;
        let h = ((maxy - miny + 2.0 * pad) * scale).ceil().max(4.0) as u32;
        let mut img = RgbaImage::new(w, h);
        for s in &strokes {
            let line: Vec<(f64, f64)> = s
                .iter()
                .map(|p| ((p.0 - minx + pad) * scale, (p.1 - miny + pad) * scale))
                .collect();
            if !line.is_empty() {
                ink_stroke(&mut img, &line, [20, 24, 60], 2.6 * scale);
            }
        }
        let path = Self::signature_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if picture::save(&img, &path).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        self.place_picture(img);
    }

    /// A dropped picture as the signature: a white ground turns transparent.
    fn signature_from(&mut self, p: &Path) {
        match picture::load(p) {
            Ok(pic) => {
                let mut img = pic.image;
                for px in img.pixels_mut() {
                    let [r, g, b, a] = px.0;
                    let light = r.min(g).min(b);
                    if light > 200 {
                        let keep = (255 - light) as u32 * 255 / 55;
                        px.0[3] = ((a as u32 * keep.min(255)) / 255) as u8;
                    }
                }
                let path = Self::signature_path();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = picture::save(&img, &path);
                self.pad = None;
                self.place_picture(img);
            }
            Err(e) => self.flash(&format!("Not a picture: {e}"), true),
        }
    }

    fn stamp_at(&self, d: &PdfDoc, x: i32, y: i32) -> Option<(usize, bool)> {
        let (page, u, v) = self.page_at(d, x, y)?;
        let grip = 10.0 / self.zoom;
        d.stamps.iter().enumerate().rev().find_map(|(i, s)| {
            if s.page != page {
                return None;
            }
            let corner = (u - (s.u + s.w)).abs() < grip && (v - (s.v + s.h)).abs() < grip;
            let inside = u >= s.u && v >= s.v && u <= s.u + s.w && v <= s.v + s.h;
            (corner || inside).then_some((i, corner))
        })
    }

    // ------------------------------------------------------------ slideshow

    pub fn toggle_slides(&mut self) {
        if let Some(s) = self.slides.take() {
            if self.x.fullscreen && !s.was_fullscreen {
                self.x.toggle_fullscreen();
            }
            self.fit = true;
            self.refit();
            self.flash("Slideshow stopped", false);
            return;
        }
        if !self.list.iter().any(|p| picture::is_picture(p)) {
            self.flash("No pictures here for a slideshow", false);
            return;
        }
        let was = self.x.fullscreen;
        if !was {
            self.x.toggle_fullscreen();
        }
        self.gallery = false;
        self.slides = Some(Slides {
            every: Duration::from_secs(4),
            next: Instant::now() + Duration::from_secs(4),
            paused: false,
            was_fullscreen: was,
        });
        self.fit = true;
        self.refit();
        self.flash(
            "Slideshow: Space pauses, + and - change the pace, Esc stops",
            false,
        );
    }

    fn slide_next(&mut self) {
        let n = self.list.len();
        for k in 1..=n {
            let i = (self.index + k) % n;
            if picture::is_picture(&self.list[i]) {
                self.index = i;
                break;
            }
        }
        if let Doc::Pic(d) = &mut self.doc {
            d.modified = false;
        }
        self.load_current();
        self.fit = true;
        if let Some(s) = &mut self.slides {
            s.next = Instant::now() + s.every;
        }
    }

    // ------------------------------------------------------------ loop

    pub fn run(&mut self) {
        let (xfd, wfd) = (self.x.fd(), self.jobs.wake_fd);
        while !self.quit {
            while let Some(ev) = self.x.poll() {
                self.event(ev);
                if self.quit {
                    return;
                }
            }
            while let Ok(done) = self.jobs.rx.try_recv() {
                self.job_done(done);
            }
            self.tick();
            if self.dirty {
                self.draw();
                self.x.present(&self.cv);
                self.dirty = false;
            }
            self.x.flush();
            let mut fds = [
                libc::pollfd {
                    fd: xfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let timeout = self.timeout();
            unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                return;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                self.jobs.drain_wake();
            }
        }
    }

    fn timeout(&self) -> i32 {
        let now = Instant::now();
        let mut t = Duration::from_secs(60);
        if let Doc::Pic(d) = &self.doc {
            if let Some(n) = d.next_frame {
                t = t.min(n.saturating_duration_since(now));
            }
        }
        if let Some(s) = &self.slides {
            if !s.paused {
                t = t.min(s.next.saturating_duration_since(now));
            }
        }
        if let Some((_, at, _)) = &self.message {
            t = t.min((*at + Duration::from_secs(4)).saturating_duration_since(now));
        }
        t.as_millis().min(60_000) as i32 + 1
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if let Doc::Pic(d) = &mut self.doc {
            if !d.frames.is_empty() && d.next_frame.is_none() && self.tool == Tool::None {
                d.next_frame = Some(now + Duration::from_millis(d.first_delay as u64));
            }
            if d.next_frame.is_some_and(|n| n <= now) {
                d.frame = (d.frame + 1) % (d.frames.len() + 1);
                let delay = if d.frame == 0 {
                    d.first_delay
                } else {
                    d.frames[d.frame - 1].1
                };
                d.next_frame = Some(now + Duration::from_millis(delay.max(20) as u64));
                d.generation += 1;
                self.dirty = true;
            }
        }
        if self
            .slides
            .as_ref()
            .is_some_and(|s| !s.paused && s.next <= now)
        {
            self.slide_next();
        }
        if self
            .message
            .as_ref()
            .is_some_and(|(_, at, _)| now.duration_since(*at) > Duration::from_secs(4))
        {
            self.message = None;
            self.dirty = true;
        }
        if self.pages_view {
            self.pages_tick();
            return;
        }
        let dpi = self.dpi();
        let v = self.view();
        if let Doc::Pdf(d) = &mut self.doc {
            // Pages on screen are asked for at this zoom; far ones are let go.
            let mut wanted = Vec::new();
            for i in 0..d.info.pages.len() {
                let (_, h) = d.info.pages[i].shown();
                let top = v.y as f64 + 20.0 + d.tops[i] * self.zoom - self.scroll;
                let bottom = top + h * self.zoom;
                if bottom >= v.y as f64 - v.h as f64 && top <= (v.y + 2 * v.h) as f64 {
                    wanted.push(i);
                }
            }
            for &i in &wanted {
                if d.pages.get(&i).is_none_or(|r| r.dpi != dpi) {
                    self.jobs.ask(Job::Page {
                        path: d.path.clone(),
                        page: i,
                        dpi,
                    });
                }
            }
            if d.pages.len() > 16 {
                d.pages.retain(|k, _| wanted.contains(k));
            }
        }
    }

    // ------------------------------------------------------------ events

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Expose(e) if e.count == 0 => self.dirty = true,
            Event::ConfigureNotify(e) if e.window == self.x.win => {
                if e.width as i32 != self.cv.w || e.height as i32 != self.cv.h {
                    self.cv.resize(e.width as i32, e.height as i32);
                    self.scaled = None;
                    self.refit();
                    self.dirty = true;
                }
            }
            Event::ClientMessage(e) => self.client_message(e),
            Event::SelectionNotify(e) if e.property == self.x.atoms.dnd_prop => {
                let data = self.x.take_drop();
                let paths = crate::x::parse_uris(&data);
                if let Some(p) = paths.first() {
                    if self.pad.is_some() && picture::is_picture(p) {
                        let p = p.clone();
                        self.signature_from(&p);
                    } else if self.pages_view && picture::is_pdf(p) {
                        for p in paths.clone() {
                            self.merge(&p);
                        }
                    } else if !self.doc.modified() {
                        self.open_paths(paths);
                        self.refit();
                    } else {
                        self.flash("Save or undo the changes first", true);
                    }
                }
                self.dirty = true;
            }
            Event::SelectionRequest(e) => {
                self.x.answer(&e, &self.clip_png, &self.clip_text);
            }
            Event::PropertyNotify(e) if e.atom == self.x.atoms.niruwm_theme => {
                if let Some(name) = self.x.root_theme() {
                    self.pal = theme::load(&theme::config_home(), &name);
                    self.dirty = true;
                }
            }
            Event::KeyPress(e) => {
                let sym = self.x.keysym(e.detail, e.state);
                self.key(sym, e.state);
            }
            Event::ButtonPress(e) => self.press(e),
            Event::ButtonRelease(e) => {
                if e.detail == 1 {
                    self.release(e.event_x as i32, e.event_y as i32);
                }
            }
            Event::MotionNotify(e) => self.motion(e.event_x as i32, e.event_y as i32),
            _ => {}
        }
    }

    fn client_message(&mut self, e: x11rb::protocol::xproto::ClientMessageEvent) {
        let a = &self.x.atoms;
        let d = e.data.as_data32();
        if e.type_ == a.wm_protocols && d[0] == a.wm_delete {
            self.want_quit();
        } else if e.type_ == a.xdnd_position {
            let source = d[0];
            self.x.send_client(
                source,
                self.x.atoms.xdnd_status,
                [self.x.win, 1, 0, 0, self.x.atoms.xdnd_copy],
            );
        } else if e.type_ == a.xdnd_drop {
            let source = d[0];
            self.x.request_drop(d[2]);
            self.x.send_client(
                source,
                self.x.atoms.xdnd_finished,
                [self.x.win, 1, self.x.atoms.xdnd_copy, 0, 0],
            );
        }
    }

    pub fn want_quit(&mut self) {
        if self.doc.modified() {
            self.ask(
                Ask::Unsaved(After::Quit),
                "Unsaved changes: S saves a copy, D discards, Esc stays",
            );
        } else {
            self.quit = true;
        }
    }

    fn key(&mut self, sym: u32, state: KeyButMask) {
        let ctrl = state.contains(KeyButMask::CONTROL);
        let shift = state.contains(KeyButMask::SHIFT);
        self.dirty = true;
        if self.prompt.is_some() {
            self.prompt_key(sym);
            return;
        }
        if self.pad.is_some() {
            match sym {
                keys::RETURN | keys::KP_ENTER => self.finish_pad(),
                keys::BACKSPACE | keys::DELETE => self.pad = Some(Vec::new()),
                keys::ESCAPE => self.pad = None,
                _ => {}
            }
            return;
        }
        if self.help && sym != 0x3f && sym != keys::F1 {
            self.help = false;
            if sym == keys::ESCAPE {
                return;
            }
        }
        if let Some(s) = &mut self.slides {
            match sym {
                0x20 => {
                    s.paused = !s.paused;
                    s.next = Instant::now() + s.every;
                    let m = if s.paused { "Paused" } else { "Playing" };
                    self.flash(m, false);
                    return;
                }
                0x2b | 0x3d | keys::KP_ADD => {
                    s.every = (s.every + Duration::from_secs(1)).min(Duration::from_secs(60));
                    let m = format!("A picture every {} s", s.every.as_secs());
                    self.flash(&m, false);
                    return;
                }
                0x2d | keys::KP_SUBTRACT => {
                    s.every = s
                        .every
                        .saturating_sub(Duration::from_secs(1))
                        .max(Duration::from_secs(1));
                    let m = format!("A picture every {} s", s.every.as_secs());
                    self.flash(&m, false);
                    return;
                }
                keys::RIGHT | keys::NEXT => {
                    self.slide_next();
                    return;
                }
                keys::LEFT | keys::PRIOR => {
                    self.step(-1);
                    self.fit = true;
                    if let Some(s) = &mut self.slides {
                        s.next = Instant::now() + s.every;
                    }
                    return;
                }
                keys::ESCAPE | 0x73 | 0x71 | 0xffc2 => {
                    self.toggle_slides();
                    return;
                }
                _ => return,
            }
        }
        if self.adjust.is_some() {
            self.adjust_key(sym, shift);
            return;
        }
        if self.find_key(sym, shift) {
            return;
        }
        if self.gallery && self.gallery_key(sym) {
            return;
        }
        if self.pages_view {
            if self.pages_key(sym, ctrl, shift) {
                return;
            }
            // Only saving, quitting, help and full screen go on from here.
            let on = (ctrl && matches!(sym, 0x73 | 0x53 | 0x71 | 0x77))
                || (!ctrl && matches!(sym, 0x3f | 0x71 | 0x66 | keys::F1 | keys::F11));
            if !on {
                self.dirty = false;
                return;
            }
        }
        if self.form && self.form_key(sym, shift) {
            return;
        }
        let is_pdf = matches!(self.doc, Doc::Pdf(_));
        let is_pic = matches!(self.doc, Doc::Pic(_));
        match (sym, ctrl) {
            (0x66, true) if is_pdf => self.open_find(),
            (0x6c, true) if is_pic => self.save_private(),
            (0x6c, false) if is_pic => self.start_adjust(Adjust::Light {
                brightness: 0,
                contrast: 0,
            }),
            (0x7a, false) if is_pic => self.start_adjust(Adjust::Straighten { deg: 0.0 }),
            (0x65, false) if is_pic => {
                let now = match self.doc.path().and_then(picture::ext).as_deref() {
                    Some("jpg" | "jpeg" | "jpe" | "jfif") => "jpg 90",
                    Some("webp") => "webp",
                    _ => "png",
                };
                self.ask_with(
                    Ask::SaveAs,
                    "Save as jpg, png or webp (jpg with a quality from 1 to 100):",
                    now.to_string(),
                );
            }
            (0x65, false) if is_pdf => self.toggle_form(),
            (0x57, false) if is_pic => self.set_wallpaper(),
            (0x70, false) if is_pdf => self.toggle_pages(),
            (0x73, true) | (0x53, true) if shift && is_pic => self.ask(
                Ask::Overwrite,
                "Replace the original? Camera details (EXIF) are not kept. Y replaces, Esc keeps it",
            ),
            (0x73, true) | (0x53, true) => {
                self.save_copy();
            }
            (0x7a, true) | (0x5a, true) => self.undo(shift),
            (0x79, true) => self.undo(true),
            (0x63, true) => self.copy_to_clipboard(),
            (0x72, true) if is_pic => self.ask(Ask::Resize, "New size (50%, 1920x1080 or a width):"),
            (0x71, true) | (0x77, true) => self.want_quit(),
            (keys::ESCAPE, _) => {
                if self.placing.take().is_some()
                    || self.crop.take().is_some()
                    || self.text_sel.take().is_some()
                    || self.find.take().is_some()
                {
                } else if !matches!(self.drag, Drag::None) {
                    self.drag = Drag::None;
                } else if self.tool != Tool::None {
                    self.tool = Tool::None;
                } else if self.info {
                    self.info = false;
                } else if let Doc::Pdf(d) = &mut self.doc {
                    if d.sel.take().is_none() {
                        self.leave_or_quit();
                    }
                } else {
                    self.leave_or_quit();
                }
            }
            (keys::RETURN | keys::KP_ENTER, _) => {
                if let (Some((a, b)), true) = (self.crop, self.tool == Tool::Crop) {
                    self.crop = None;
                    self.edit(|img| picture::clamp_rect(img, a, b).map(|r| picture::crop(img, r)));
                    self.fit = true;
                    self.refit();
                } else if self.gallery {
                    self.gallery = false;
                }
            }
            (0x71, false) => self.want_quit(),
            (keys::F1, _) | (0x3f, _) => self.help = !self.help,
            (0x69, false) => {
                self.info = !self.info;
                if let Doc::Pic(d) = &mut self.doc {
                    if self.info && d.xmp_gps.is_none() && d.facts.place.is_none() {
                        d.xmp_gps = Some(picture::xmp_location(&d.path));
                    }
                }
            }
            (0x66, false) | (keys::F11, _) => {
                self.x.toggle_fullscreen();
            }
            (0x67, false) => self.toggle_gallery(),
            (0xffc2, _) => self.toggle_slides(),
            (0x73, false) if is_pic => self.toggle_slides(),
            (0x73, false) if is_pdf => self.start_signature(false),
            (0x53, false) if is_pdf => self.start_signature(true),
            (0x74, false) if is_pdf => self.ask(Ask::PdfText, "Text to put on the page:"),
            (0x64, false) if is_pdf => self.place_text(today(), 11.0),
            (0x78, false) if is_pdf => self.place_text("X".into(), 14.0),
            (0x6f, false) => self.resort(),
            (keys::DELETE, _) => {
                let removed = if let Doc::Pdf(d) = &mut self.doc {
                    d.sel.take().map(|i| d.stamps.remove(i)).is_some()
                } else {
                    false
                };
                if !removed && self.current().is_some() {
                    let name = name_of(self.current().unwrap());
                    self.ask(Ask::Trash, &format!("Move {name} to the trash? Y moves it, Esc keeps it"));
                }
            }
            (keys::RIGHT | keys::LEFT | keys::UP | keys::DOWN, _) if self.nudge(sym, shift) => {}
            (keys::RIGHT | keys::NEXT | 0x20, _) if is_pdf => self.page_step(1, sym == keys::RIGHT),
            (keys::LEFT | keys::PRIOR | keys::BACKSPACE, _) if is_pdf => self.page_step(-1, sym == keys::LEFT),
            (keys::DOWN, _) if is_pdf => self.scroll_by(60.0),
            (keys::UP, _) if is_pdf => self.scroll_by(-60.0),
            (keys::RIGHT | keys::NEXT | 0x20, _) => self.step(1),
            (keys::LEFT | keys::PRIOR | keys::BACKSPACE, _) => self.step(-1),
            (0x5d, _) => self.step(1),
            (0x5b, _) => self.step(-1),
            (keys::HOME, _) if is_pdf => self.scroll_to_page(0),
            (keys::END, _) if is_pdf => self.scroll_to_page(usize::MAX),
            (keys::HOME, _) => self.go(0),
            (keys::END, _) => self.go(usize::MAX),
            (0x2b | 0x3d | keys::KP_ADD, _) => self.set_zoom(self.zoom * 1.25),
            (0x2d | keys::KP_SUBTRACT, _) => self.set_zoom(self.zoom / 1.25),
            (0x30, false) => {
                self.fit = true;
                self.refit();
            }
            (0x31, false) => self.set_zoom(1.0),
            (0x77, false) if is_pdf => {
                self.fit = true;
                self.refit();
            }
            (0x72, false) if is_pic => self.edit(|i| Some(picture::rotate(i, 1))),
            (0x52, false) if is_pic => self.edit(|i| Some(picture::rotate(i, 3))),
            (0x68, false) if is_pic => self.edit(|i| Some(picture::flip(i, true))),
            (0x76, false) if is_pic => self.edit(|i| Some(picture::flip(i, false))),
            (0x63, false) if is_pic => self.set_tool(Tool::Crop),
            (0x70, false) if is_pic => self.set_tool(Tool::Pen),
            (0x61, false) if is_pic => self.set_tool(Tool::Arrow),
            (0x62, false) if is_pic => self.set_tool(Tool::Box),
            (0x74, false) if is_pic => self.set_tool(Tool::Text),
            (0x78, false) if is_pic => self.set_tool(Tool::Blur),
            (0x6b, false) => {
                self.ink = (self.ink + 1) % INKS.len();
                let m = format!("Ink: {}", INKS[self.ink].1);
                self.flash(&m, false);
            }
            (0x2c, false) => {
                self.width = (self.width - 1.0).max(1.0);
                let m = format!("Line {} px", self.width);
                self.flash(&m, false);
            }
            (0x2e, false) => {
                self.width = (self.width + 1.0).min(40.0);
                let m = format!("Line {} px", self.width);
                self.flash(&m, false);
            }
            _ => self.dirty = false,
        }
    }

    fn leave_or_quit(&mut self) {
        if self.gallery {
            self.gallery = false;
        } else if self.x.fullscreen {
            self.x.toggle_fullscreen();
        } else {
            self.want_quit();
        }
    }

    fn set_tool(&mut self, t: Tool) {
        self.tool = if self.tool == t { Tool::None } else { t };
        self.crop = None;
        if let Doc::Pic(d) = &mut self.doc {
            d.next_frame = None;
        }
        if self.tool != Tool::None {
            let m = format!(
                "{}  (K ink, comma and period line width, Esc done)",
                self.tool.label()
            );
            self.flash(&m, false);
        }
    }

    /// Arrows move a selected stamp (Shift: ten times as far).
    fn nudge(&mut self, sym: u32, shift: bool) -> bool {
        let Doc::Pdf(d) = &mut self.doc else {
            return false;
        };
        let Some(i) = d.sel else { return false };
        let step = if shift { 10.0 } else { 1.0 };
        let s = &mut d.stamps[i];
        match sym {
            keys::RIGHT => s.u += step,
            keys::LEFT => s.u -= step,
            keys::UP => s.v -= step,
            keys::DOWN => s.v += step,
            _ => return false,
        }
        true
    }

    fn page_step(&mut self, d: i64, arrow: bool) {
        let Doc::Pdf(doc) = &self.doc else { return };
        let n = doc.info.pages.len() as i64;
        let cur = self.current_page(doc) as i64;
        let next = cur + d;
        if next < 0 || next >= n {
            // Past the ends, the arrows go on to the next file.
            if arrow {
                self.step(d);
            }
            return;
        }
        self.scroll_to_page(next as usize);
    }

    fn scroll_by(&mut self, dy: f64) {
        self.scroll += dy;
        self.clamp_pan();
        self.dirty = true;
    }

    fn prompt_key(&mut self, sym: u32) {
        let Some(mut p) = self.prompt.take() else {
            return;
        };
        let yes_no = matches!(p.ask, Ask::Overwrite | Ask::Trash);
        if let Ask::Unsaved(after) = p.ask {
            match sym {
                0x73 | 0x53 => {
                    if self.save_copy() {
                        self.continue_after(after);
                    }
                }
                0x64 | 0x44 => {
                    match &mut self.doc {
                        Doc::Pic(d) => d.modified = false,
                        Doc::Pdf(d) => d.discard(),
                        _ => {}
                    }
                    self.continue_after(after);
                }
                _ => {}
            }
            return;
        }
        if yes_no {
            if matches!(sym, 0x79 | 0x59 | 0x6a | 0x4a) {
                match p.ask {
                    Ask::Overwrite => self.overwrite(),
                    Ask::Trash => self.trash_current(),
                    _ => {}
                }
            }
            return;
        }
        match sym {
            keys::ESCAPE => {}
            keys::RETURN | keys::KP_ENTER => self.submit(p),
            keys::BACKSPACE => {
                p.text.pop();
                self.prompt = Some(p);
            }
            _ => {
                if let Some(c) = keysym_char(sym) {
                    p.text.push(c);
                }
                self.prompt = Some(p);
            }
        }
    }

    fn continue_after(&mut self, after: After) {
        match after {
            After::Quit => self.quit = true,
            After::LeavePages => {
                if self.pages_view {
                    self.leave_pages();
                }
            }
            After::Open(i) => {
                self.index = i.min(self.list.len().saturating_sub(1));
                self.load_current();
                self.refit();
            }
        }
    }

    fn submit(&mut self, p: Prompt) {
        let text = p.text.trim().to_string();
        match p.ask {
            Ask::Resize => {
                let Doc::Pic(d) = &self.doc else { return };
                match picture::parse_size(&text, d.image.width(), d.image.height()) {
                    Some((w, h)) => {
                        self.edit(|i| Some(picture::resize(i, w, h)));
                        self.fit = true;
                        self.refit();
                        self.flash(&format!("Now {w} x {h}"), false);
                    }
                    None => self.flash("Write 50%, 1920x1080 or a width", true),
                }
            }
            Ask::PicText { at } if !text.is_empty() => {
                let ink = self.ink_rgba();
                let px = (self.width * 6.0 + 12.0) / self.zoom.max(0.05);
                let size = self.sans.size(px as f32);
                let mut img = match &self.doc {
                    Doc::Pic(d) => d.image.clone(),
                    _ => return,
                };
                self.sans.draw_rgba(
                    &mut img,
                    at.0 as i32,
                    at.1 as i32 - size.ascent / 2,
                    &text,
                    size,
                    ink,
                );
                self.edit(|_| Some(img));
            }
            Ask::PdfText if !text.is_empty() => self.place_text(text, 11.0),
            Ask::Field(i) => self.field_submit(i, &p.text),
            Ask::SaveAs => self.save_as(&text),
            Ask::Merge if !text.is_empty() => self.merge(Path::new(&text)),
            Ask::Extract if !text.is_empty() => self.extract(&text),
            _ => {}
        }
    }

    fn toggle_gallery(&mut self) {
        self.gallery = !self.gallery;
        if !self.gallery {
            self.jobs.drop_where(|j| matches!(j, Job::Thumb { .. }));
        }
        self.dirty = true;
    }

    /// Keys of the gallery; false when the key is not one of them.
    fn gallery_key(&mut self, sym: u32) -> bool {
        let cols = self.gallery_cols() as i64;
        let n = self.list.len() as i64;
        if n == 0 {
            return false;
        }
        let i = self.index as i64;
        let to = match sym {
            keys::RIGHT => i + 1,
            keys::LEFT => i - 1,
            keys::DOWN => i + cols,
            keys::UP => i - cols,
            keys::NEXT => i + cols * 3,
            keys::PRIOR => i - cols * 3,
            keys::HOME => 0,
            keys::END => n - 1,
            keys::RETURN | keys::KP_ENTER | 0x20 => {
                self.gallery = false;
                self.refit();
                return true;
            }
            keys::ESCAPE | 0x67 => {
                self.gallery = false;
                return true;
            }
            _ => return false,
        };
        let to = to.clamp(0, n - 1) as usize;
        if to != self.index {
            self.go(to);
        }
        self.gallery_follow();
        true
    }

    pub fn gallery_cell(&self) -> (i32, i32) {
        let th = self.ui.size(self.font_px).height;
        (176, 176 + th + 8)
    }

    pub fn gallery_cols(&self) -> i32 {
        let (cw, _) = self.gallery_cell();
        ((self.view().w - 16) / (cw + 12)).max(1)
    }

    fn gallery_follow(&mut self) {
        let (_, ch) = self.gallery_cell();
        let row = self.index as i32 / self.gallery_cols();
        let v = self.view();
        let top = row * (ch + 12);
        if top < self.gallery_scroll {
            self.gallery_scroll = top;
        } else if top + ch + 24 > self.gallery_scroll + v.h {
            self.gallery_scroll = top + ch + 24 - v.h;
        }
    }

    fn gallery_hit(&self, x: i32, y: i32) -> Option<usize> {
        let (cw, ch) = self.gallery_cell();
        let v = self.view();
        let cols = self.gallery_cols();
        let left = v.x + (v.w - cols * (cw + 12) + 12) / 2;
        let (gx, gy) = (x - left, y - v.y - 12 + self.gallery_scroll);
        if gx < 0 || gy < 0 {
            return None;
        }
        let (c, r) = (gx / (cw + 12), gy / (ch + 12));
        if c >= cols || gx % (cw + 12) >= cw {
            return None;
        }
        let i = (r * cols + c) as usize;
        (i < self.list.len()).then_some(i)
    }

    fn press(&mut self, e: ButtonPressEvent) {
        let (x, y) = (e.event_x as i32, e.event_y as i32);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        self.mouse = (x, y);
        self.dirty = true;
        if self.help {
            self.help = false;
            return;
        }
        match e.detail {
            4 | 5 => {
                let up = e.detail == 4;
                if self.pages_view {
                    self.strip_scroll = (self.strip_scroll + if up { -80 } else { 80 }).max(0);
                } else if self.gallery {
                    self.gallery_scroll = (self.gallery_scroll + if up { -80 } else { 80 }).max(0);
                } else if matches!(self.doc, Doc::Pdf(_)) && !ctrl {
                    self.scroll_by(if up { -80.0 } else { 80.0 });
                } else if self.slides.is_some() {
                    self.step(if up { -1 } else { 1 });
                } else {
                    self.zoom_at(if up { 1.15 } else { 1.0 / 1.15 }, (x, y));
                }
                return;
            }
            8 => {
                self.step(-1);
                return;
            }
            9 => {
                self.step(1);
                return;
            }
            3 => {
                self.tool = Tool::None;
                self.placing = None;
                self.crop = None;
                return;
            }
            1 => {}
            _ => return,
        }
        if self.pad.is_some() {
            let r = self.pad_rect();
            if r.contains(x, y) {
                if let Some(p) = &mut self.pad {
                    p.push(vec![((x - r.x) as f64, (y - r.y) as f64)]);
                }
                self.drag = Drag::Pad;
            }
            return;
        }
        if self.gallery {
            if let Some(i) = self.gallery_hit(x, y) {
                if i == self.index {
                    self.gallery = false;
                    self.refit();
                } else {
                    self.go(i);
                }
            }
            return;
        }
        if self.pages_view {
            self.pages_click(x, y);
            return;
        }
        if self.form && self.form_click(x, y) {
            return;
        }
        if let Some(pl) = self.placing.take() {
            if let Doc::Pdf(d) = &mut self.doc {
                d.stamps.push(pl.stamp);
                d.sel = Some(d.stamps.len() - 1);
                self.flash("Drag to move, the corner to resize, Delete removes; Ctrl+S saves a signed copy", false);
            }
            return;
        }
        match &self.doc {
            Doc::Pdf(d) => {
                let hit = self.stamp_at(d, x, y);
                let at = self.page_at(d, x, y);
                let press = Drag::Press {
                    x,
                    y,
                    ox: self.ox,
                    oy: self.oy,
                    scroll: self.scroll,
                };
                if hit.is_none() && e.state.contains(KeyButMask::SHIFT) {
                    if let Some((page, u, v)) = at {
                        self.start_select(page, u, v);
                        return;
                    }
                }
                self.text_sel = None;
                let Doc::Pdf(d) = &mut self.doc else { return };
                if let Some((i, corner)) = hit {
                    d.sel = Some(i);
                    if corner {
                        self.drag = Drag::Resize { idx: i };
                    } else if let Some((_, u, v)) = at {
                        let s = &d.stamps[i];
                        self.drag = Drag::Move {
                            idx: i,
                            du: u - s.u,
                            dv: v - s.v,
                        };
                    }
                } else {
                    d.sel = None;
                    self.drag = press;
                }
            }
            Doc::Pic(_) => {
                let p = self.to_image(x, y);
                self.drag = match self.tool {
                    Tool::None => Drag::Press {
                        x,
                        y,
                        ox: self.ox,
                        oy: self.oy,
                        scroll: self.scroll,
                    },
                    Tool::Pen => Drag::Line(vec![p]),
                    Tool::Text => {
                        self.ask(Ask::PicText { at: p }, "Text:");
                        Drag::None
                    }
                    _ => Drag::Rect(p, p),
                };
            }
            _ => {}
        }
    }

    fn motion(&mut self, x: i32, y: i32) {
        self.mouse = (x, y);
        if self.placing.is_some() {
            self.follow(x, y);
            return;
        }
        let zoom = self.zoom;
        match &mut self.drag {
            Drag::None => return,
            Drag::Select => {
                self.select_to(x, y);
                return;
            }
            Drag::Press {
                x: px,
                y: py,
                ox,
                oy,
                scroll,
            } => {
                if (x - *px).abs() + (y - *py).abs() > 4 {
                    self.drag = Drag::Pan {
                        x: *px,
                        y: *py,
                        ox: *ox,
                        oy: *oy,
                        scroll: *scroll,
                    };
                    self.motion(x, y);
                }
                return;
            }
            Drag::Pan {
                x: px,
                y: py,
                ox,
                oy,
                scroll,
            } => {
                let (dx, dy) = ((x - *px) as f64, (y - *py) as f64);
                let (nx, ny, ns) = (*ox + dx, *oy + dy, *scroll - dy);
                self.fit = false;
                match self.doc {
                    Doc::Pdf(_) => {
                        self.scroll = ns;
                        self.ox = nx;
                    }
                    _ => {
                        self.ox = nx;
                        self.oy = ny;
                    }
                }
                self.clamp_pan();
            }
            Drag::Line(points) => {
                let p = ((x as f64 - self.ox) / zoom, (y as f64 - self.oy) / zoom);
                if points
                    .last()
                    .is_none_or(|l| (l.0 - p.0).abs() + (l.1 - p.1).abs() > 0.5 / zoom)
                {
                    points.push(p);
                }
            }
            Drag::Rect(_, b) => {
                *b = ((x as f64 - self.ox) / zoom, (y as f64 - self.oy) / zoom);
            }
            Drag::Pad => {
                let r = self.pad_rect();
                if let Some(s) = self.pad.as_mut().and_then(|p| p.last_mut()) {
                    s.push((
                        (x - r.x).clamp(0, r.w) as f64,
                        (y - r.y).clamp(0, r.h) as f64,
                    ));
                }
            }
            Drag::Move { idx, du, dv } => {
                let (idx, du, dv) = (*idx, *du, *dv);
                let hit = match &self.doc {
                    Doc::Pdf(d) => self.page_at(d, x, y),
                    _ => None,
                };
                if let (Doc::Pdf(d), Some((page, u, v))) = (&mut self.doc, hit) {
                    let s = &mut d.stamps[idx];
                    s.page = page;
                    s.u = u - du;
                    s.v = v - dv;
                }
            }
            Drag::Resize { idx } => {
                let idx = *idx;
                let hit = match &self.doc {
                    Doc::Pdf(d) => self.page_at(d, x, y),
                    _ => None,
                };
                if let (Doc::Pdf(d), Some((_, u, v))) = (&mut self.doc, hit) {
                    let s = &mut d.stamps[idx];
                    let w = (u - s.u).max(8.0);
                    match &mut s.kind {
                        pdf::Kind::Picture { w: iw, h: ih, .. } => {
                            s.w = w;
                            s.h = w * *ih as f64 / (*iw).max(1) as f64;
                        }
                        pdf::Kind::Text { text, size } => {
                            let h = (v - s.v).max(6.0);
                            *size = (h / pdf::TEXT_HEIGHT).clamp(4.0, 96.0);
                            s.h = *size * pdf::TEXT_HEIGHT;
                            s.w = pdf::helvetica_width(text, *size);
                        }
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn release(&mut self, _x: i32, _y: i32) {
        if matches!(self.drag, Drag::Select) {
            self.drag = Drag::None;
            self.end_select();
            return;
        }
        if matches!(self.drag, Drag::Pad) {
            self.drag = Drag::None;
            return;
        }
        if matches!(self.drag, Drag::Pan { .. }) {
            self.drag = Drag::None;
            return;
        }
        self.finish_drag();
    }
}

fn pic_doc(path: PathBuf, pic: picture::Picture) -> PicDoc {
    PicDoc {
        path,
        image: pic.image,
        frames: pic.frames,
        first_delay: pic.first_delay,
        frame: 0,
        next_frame: None,
        facts: pic.facts,
        undo: Vec::new(),
        redo: Vec::new(),
        modified: false,
        generation: 1,
        xmp_gps: None,
    }
}

/// A pen line with the soft edge of ink, onto a transparent picture.
fn ink_stroke(img: &mut RgbaImage, line: &[(f64, f64)], rgb: [u8; 3], width: f64) {
    let mut cover: HashMap<(i32, i32), f32> = HashMap::new();
    let segs: Vec<((f64, f64), (f64, f64))> = if line.len() == 1 {
        vec![(line[0], line[0])]
    } else {
        line.windows(2).map(|w| (w[0], w[1])).collect()
    };
    for (a, b) in segs {
        picture::brush_segment(a, b, width / 2.0, |x, y, c| {
            let e = cover.entry((x, y)).or_insert(0.0);
            if c > *e {
                *e = c;
            }
        });
    }
    for ((x, y), c) in cover {
        if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
            let p = img.get_pixel_mut(x as u32, y as u32);
            let a = (c * 255.0) as u8;
            if a > p.0[3] {
                *p = image::Rgba([rgb[0], rgb[1], rgb[2], a]);
            }
        }
    }
}

pub fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

pub fn today() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        )
    }
}

/// The character a keysym types (Latin-1 and Unicode keysyms).
pub fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        0x0100_0000..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        _ => None,
    }
}
