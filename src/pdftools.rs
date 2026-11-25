//! What the viewer does with a PDF besides signing it: find text (Ctrl+F),
//! select and copy it (Shift and drag, Ctrl+C), fill in its form (E), and
//! the pages view (P) where pages are turned, removed, moved, added from
//! another PDF and saved, all or a range, as a new file. Everything here
//! starts when it is asked for; nothing is read when a PDF opens.
use crate::app::{name_of, App, Ask, Doc, PageRaster, PdfDoc};
use crate::canvas::Rect;
use crate::forms::{self, Kind};
use crate::jobs::Job;
use crate::pdfpages::{self, Plan};
use crate::pdftext;
use crate::x::keys;
use std::path::{Path, PathBuf};

/// Thumbnails of the pages view are drawn at this DPI (an A4 page is 149 x
/// 211 pixels); no zoom of the document view uses it.
pub const STRIP_DPI: u32 = 18;
pub const F3: u32 = 0xffc0;
pub const TAB: u32 = 0xff09;
pub const ISO_LEFT_TAB: u32 = 0xfe20;

pub struct Find {
    pub text: String,
    pub hits: Vec<pdftext::Hit>,
    pub cur: usize,
    /// The bar takes the keys (closed: F3 still goes on).
    pub open: bool,
}

/// Text chosen with Shift and a drag, from `a` to `b` on one page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSel {
    pub page: usize,
    pub a: (f64, f64),
    pub b: (f64, f64),
}

impl App {
    fn pdf(&self) -> Option<&PdfDoc> {
        match &self.doc {
            Doc::Pdf(d) => Some(d),
            _ => None,
        }
    }

    fn pdf_mut(&mut self) -> Option<&mut PdfDoc> {
        match &mut self.doc {
            Doc::Pdf(d) => Some(d),
            _ => None,
        }
    }

    /// Ask for the words of the PDF once; false while they are not there.
    pub fn want_words(&mut self) -> bool {
        let Some(d) = self.pdf_mut() else {
            return false;
        };
        if d.words.is_some() {
            return true;
        }
        if !d.words_asked {
            d.words_asked = true;
            let path = d.path.clone();
            self.jobs.ask(Job::Words { path });
        }
        false
    }

    pub fn words_done(&mut self, path: PathBuf, r: Result<pdftext::Words, String>) {
        let Some(d) = self.pdf_mut() else { return };
        if d.path != path {
            return;
        }
        match r {
            Ok(w) => {
                let none = w.iter().all(|p| p.is_empty());
                d.words = Some(w);
                if none {
                    self.flash(
                        "This PDF has no text to find (a scan is a picture of text)",
                        false,
                    );
                }
                if self.find.is_some() {
                    self.find_update(true);
                }
            }
            Err(e) => {
                d.words_asked = false;
                self.flash(&format!("Cannot read the text: {e}"), true);
            }
        }
    }

    // ------------------------------------------------------------ find

    pub fn open_find(&mut self) {
        if self.pdf().is_none() {
            return;
        }
        match &mut self.find {
            Some(f) => f.open = true,
            None => {
                self.find = Some(Find {
                    text: String::new(),
                    hits: Vec::new(),
                    cur: 0,
                    open: true,
                })
            }
        }
        self.want_words();
        self.dirty = true;
    }

    /// Keys while the bar is open; false for keys it leaves to the view.
    pub fn find_key(&mut self, sym: u32, shift: bool) -> bool {
        let Some(f) = &mut self.find else {
            return false;
        };
        if !f.open {
            if sym == F3 && !f.hits.is_empty() {
                self.find_go(if shift { -1 } else { 1 });
                return true;
            }
            return false;
        }
        match sym {
            keys::ESCAPE => {
                self.find = None;
            }
            keys::RETURN | keys::KP_ENTER | F3 => self.find_go(if shift { -1 } else { 1 }),
            keys::BACKSPACE => {
                f.text.pop();
                self.find_update(true);
            }
            _ => match crate::app::keysym_char(sym) {
                Some(c) => {
                    f.text.push(c);
                    self.find_update(true);
                }
                None => return false,
            },
        }
        self.dirty = true;
        true
    }

    /// Search again; `jump` goes to the first place from the page on screen.
    fn find_update(&mut self, jump: bool) {
        let here = self.pdf().map(|d| self.current_page(d)).unwrap_or(0);
        let (Some(f), Some(Doc::Pdf(d))) = (self.find.as_mut(), Some(&self.doc)) else {
            return;
        };
        let Some(words) = &d.words else { return };
        f.hits = pdftext::find(words, &f.text);
        f.cur = f.hits.iter().position(|h| h.page >= here).unwrap_or(0);
        if jump && !f.hits.is_empty() {
            self.show_hit();
        }
        self.dirty = true;
    }

    fn find_go(&mut self, d: i64) {
        let Some(f) = &mut self.find else { return };
        if f.hits.is_empty() {
            if !f.text.is_empty() {
                self.flash("Not found", false);
            }
            return;
        }
        let n = f.hits.len() as i64;
        f.cur = (f.cur as i64 + d).rem_euclid(n) as usize;
        self.show_hit();
    }

    /// Scroll so the current place found is on screen, a third from the top.
    fn show_hit(&mut self) {
        let Some(hit) = self.find.as_ref().and_then(|f| f.hits.get(f.cur)).cloned() else {
            return;
        };
        let Some(b) = hit.boxes.first().copied() else {
            return;
        };
        let v = self.view();
        let Doc::Pdf(d) = &self.doc else { return };
        let top = d.tops[hit.page] + b[1];
        let (_, y, _, _) = self.page_rect(d, hit.page);
        let screen_y = y + b[1] * self.zoom;
        if screen_y < v.y as f64 + 20.0 || screen_y > (v.y + v.h) as f64 - 40.0 {
            self.scroll = top * self.zoom + 20.0 - v.h as f64 / 3.0;
        }
        let (x, _, _, _) = self.page_rect(d, hit.page);
        let sx = x + b[0] * self.zoom;
        if sx < v.x as f64 || sx + (b[2] - b[0]) * self.zoom > (v.x + v.w) as f64 {
            self.ox -= sx - (v.x + v.w / 3) as f64;
        }
        self.fit = false;
        self.clamp_pan();
        self.dirty = true;
    }

    /// The bottom bar while finding: "Find: text   2 of 7".
    pub fn find_status(&self) -> Option<(String, String)> {
        let f = self.find.as_ref()?;
        let words = self.pdf().is_some_and(|d| d.words.is_some());
        let state = if !words {
            "reading the text…".to_string()
        } else if f.text.trim().is_empty() {
            "Enter next, Shift+Enter previous, Esc closes".into()
        } else if f.hits.is_empty() {
            "not found".into()
        } else {
            format!("{} of {}", f.cur + 1, f.hits.len())
        };
        Some((f.text.clone(), state))
    }

    // ------------------------------------------------------------ select

    /// The words chosen: their page and first and last word.
    pub fn sel_range(&self) -> Option<(usize, usize, usize)> {
        let s = self.text_sel?;
        let ws = self.pdf()?.words.as_ref()?.get(s.page)?;
        let (a, b) = pdftext::range(ws, s.a, s.b)?;
        Some((s.page, a, b))
    }

    pub fn start_select(&mut self, page: usize, u: f64, v: f64) {
        self.text_sel = Some(TextSel {
            page,
            a: (u, v),
            b: (u, v),
        });
        self.drag = crate::app::Drag::Select;
        self.want_words();
    }

    /// The pointer on the page being selected on (outside it too).
    pub fn select_to(&mut self, x: i32, y: i32) {
        let Some(s) = self.text_sel else { return };
        let Doc::Pdf(d) = &self.doc else { return };
        let (px, py, _, _) = self.page_rect(d, s.page);
        let b = ((x as f64 - px) / self.zoom, (y as f64 - py) / self.zoom);
        if let Some(s) = &mut self.text_sel {
            s.b = b;
        }
        self.dirty = true;
    }

    pub fn end_select(&mut self) {
        let Some(s) = self.text_sel else { return };
        if (s.a.0 - s.b.0).abs() + (s.a.1 - s.b.1).abs() < 2.0 {
            self.text_sel = None;
        } else {
            self.flash("Ctrl+C copies the text", false);
        }
    }

    pub fn copy_text(&mut self) {
        if self.text_sel.is_none() {
            self.flash("Hold Shift and drag over the text to choose it", false);
            return;
        }
        if !self.want_words() {
            self.flash("Reading the text… press Ctrl+C again", false);
            return;
        }
        let Some((page, a, b)) = self.sel_range() else {
            self.flash("No text there", false);
            return;
        };
        let Some(ws) = self.pdf().and_then(|d| d.words.as_ref()).map(|w| &w[page]) else {
            return;
        };
        self.clip_text = pdftext::text_of(ws, a, b);
        self.clip_png.clear();
        self.x.own_clipboard();
        let n = self.clip_text.split_whitespace().count();
        self.flash(
            &format!("{n} word{} on the clipboard", if n == 1 { "" } else { "s" }),
            false,
        );
    }

    // ------------------------------------------------------------ forms

    pub fn toggle_form(&mut self) {
        if self.form {
            self.form = false;
            self.field_sel = None;
            return;
        }
        let Some(d) = self.pdf_mut() else { return };
        if d.fields.is_none() {
            match forms::read(&d.path, &d.info.pages) {
                Ok(f) => d.fields = Some(f),
                Err(e) => {
                    self.flash(&format!("Cannot read the form: {e}"), true);
                    return;
                }
            }
        }
        let n = d.fields.as_ref().map_or(0, |f| f.len());
        if n == 0 {
            self.flash("This PDF has no form to fill in", false);
            return;
        }
        self.form = true;
        self.field_sel = None;
        self.flash(
            &format!(
                "{n} field{}: click one or Tab to it, Enter or Space fills it in; Ctrl+S saves a filled copy",
                if n == 1 { "" } else { "s" }
            ),
            false,
        );
    }

    /// The field and widget under a screen point.
    pub fn field_at(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        let d = self.pdf()?;
        let (page, u, v) = self.page_at(d, x, y)?;
        d.fields.as_ref()?.iter().enumerate().find_map(|(i, f)| {
            f.widgets
                .iter()
                .position(|w| {
                    w.page == page
                        && u >= w.rect[0]
                        && u <= w.rect[2]
                        && v >= w.rect[1]
                        && v <= w.rect[3]
                })
                .map(|k| (i, k))
        })
    }

    /// A click in form mode: true when it was on a field.
    pub fn form_click(&mut self, x: i32, y: i32) -> bool {
        let Some((i, k)) = self.field_at(x, y) else {
            return false;
        };
        self.field_sel = Some(i);
        self.fill_field(i, k);
        true
    }

    /// Fill in field `i` (its widget `k`): a box is ticked or cleared, a
    /// radio button chosen, text and choices asked for.
    fn fill_field(&mut self, i: usize, k: usize) {
        let Some(f) = self
            .pdf_mut()
            .and_then(|d| d.fields.as_mut())
            .and_then(|f| f.get_mut(i))
        else {
            return;
        };
        if f.read_only {
            let m = format!("{} cannot be changed", f.name);
            self.flash(&m, false);
            return;
        }
        match &f.kind {
            Kind::Check => {
                let on = f.widgets[k].on.clone().unwrap_or_else(|| "Yes".into());
                f.value = if f.value == on { "Off".into() } else { on };
                f.changed = true;
            }
            Kind::Radio => {
                if let Some(on) = f.widgets[k].on.clone() {
                    f.value = on;
                    f.changed = true;
                }
            }
            Kind::Text { .. } => {
                let (label, text) = (format!("{}:", f.name), f.value.clone());
                self.ask_with(Ask::Field(i), &label, text);
            }
            Kind::Choice { options, .. } => {
                let list: Vec<String> = options
                    .iter()
                    .enumerate()
                    .map(|(n, (_, s))| format!("{} {s}", n + 1))
                    .collect();
                let label = format!("{} ({}):", f.name, list.join(", "));
                let text = forms::shown_value(f);
                self.ask_with(Ask::Field(i), &label, text);
            }
        }
        self.dirty = true;
    }

    pub fn field_submit(&mut self, i: usize, text: &str) {
        let Some(f) = self
            .pdf_mut()
            .and_then(|d| d.fields.as_mut())
            .and_then(|f| f.get_mut(i))
        else {
            return;
        };
        match forms::choose(&f.kind, text) {
            Some(v) => {
                if v != f.value {
                    f.value = v;
                    f.changed = true;
                }
            }
            None => {
                let m = format!("{} has no such choice", f.name);
                self.flash(&m, true);
            }
        }
    }

    /// Keys in form mode; false for keys it leaves to the view.
    pub fn form_key(&mut self, sym: u32, shift: bool) -> bool {
        let n = self
            .pdf()
            .and_then(|d| d.fields.as_ref())
            .map_or(0, |f| f.len());
        if n == 0 {
            return false;
        }
        match sym {
            TAB | ISO_LEFT_TAB => {
                let back = shift || sym == ISO_LEFT_TAB;
                let next = match self.field_sel {
                    None if back => n - 1,
                    None => 0,
                    Some(i) if back => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                };
                self.field_sel = Some(next);
                self.show_field(next);
            }
            keys::RETURN | keys::KP_ENTER | 0x20 => {
                let Some(i) = self.field_sel else {
                    return false;
                };
                self.fill_field(i, 0);
            }
            keys::ESCAPE => {
                if self.field_sel.take().is_none() {
                    self.form = false;
                }
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    fn show_field(&mut self, i: usize) {
        let Some(w) = self
            .pdf()
            .and_then(|d| d.fields.as_ref())
            .and_then(|f| f.get(i))
            .and_then(|f| f.widgets.first())
            .cloned()
        else {
            return;
        };
        let v = self.view();
        let Doc::Pdf(d) = &self.doc else { return };
        let (_, y, _, _) = self.page_rect(d, w.page);
        let (top, bottom) = (y + w.rect[1] * self.zoom, y + w.rect[3] * self.zoom);
        if top < v.y as f64 || bottom > (v.y + v.h) as f64 {
            self.scroll = (d.tops[w.page] + w.rect[1]) * self.zoom + 20.0 - v.h as f64 / 3.0;
            self.clamp_pan();
        }
    }

    // ------------------------------------------------------------ pages view

    pub fn toggle_pages(&mut self) {
        if self.pages_view {
            self.leave_pages();
            return;
        }
        let Some(d) = self.pdf() else { return };
        if !d.stamps.is_empty() || d.fields_changed() {
            self.flash(
                "Save the signature or the form first (Ctrl+S), then edit the pages",
                false,
            );
            return;
        }
        let cur = self.current_page(d);
        let n = d.info.pages.len();
        let path = d.path.clone();
        if let Some(d) = self.pdf_mut() {
            if d.plan.is_none() {
                d.plan = Some(Plan::new(path, n));
            }
        }
        self.pages_view = true;
        self.page_sel = cur;
        self.form = false;
        self.find = None;
        self.text_sel = None;
        self.pages_follow();
        self.flash(
            "Pages: R turns, Delete removes, Shift+arrows move, M adds a PDF, E saves pages apart, Ctrl+S saves",
            false,
        );
    }

    /// Back to the document; with page changes not saved, asks first.
    pub fn leave_pages(&mut self) {
        let changed = self.pdf().is_some_and(|d| d.plan_changed());
        if changed {
            self.ask(
                Ask::Unsaved(crate::app::After::LeavePages),
                "Page changes not saved: S saves a copy, D discards, Esc stays",
            );
            return;
        }
        let sel = self.page_sel;
        self.pages_view = false;
        if let Some(d) = self.pdf_mut() {
            d.plan = None;
            d.plan_undo.clear();
            d.strip.clear();
        }
        self.scroll_to_page(sel);
        self.jobs
            .drop_where(|j| matches!(j, Job::Page { dpi, .. } if *dpi == STRIP_DPI));
    }

    fn plan_len(&self) -> usize {
        self.pdf()
            .and_then(|d| d.plan.as_ref())
            .map_or(0, |p| p.sheets.len())
    }

    /// Change the plan, keeping the old one to undo.
    fn plan_edit(&mut self, f: impl FnOnce(&mut Plan, usize) -> Option<usize>) {
        let sel = self.page_sel;
        let Some(d) = self.pdf_mut() else { return };
        let Some(plan) = d.plan.as_mut() else { return };
        let before = plan.clone();
        match f(plan, sel) {
            Some(next) => {
                d.plan_undo.push(before);
                if d.plan_undo.len() > 50 {
                    d.plan_undo.remove(0);
                }
                self.page_sel = next;
            }
            None => *plan = before,
        }
        self.pages_follow();
        self.dirty = true;
    }

    pub fn pages_key(&mut self, sym: u32, ctrl: bool, shift: bool) -> bool {
        let n = self.plan_len();
        if n == 0 {
            return false;
        }
        let cols = self.strip_cols() as i64;
        let i = self.page_sel as i64;
        let to = |d: i64| (i + d).clamp(0, n as i64 - 1) as usize;
        match (sym, ctrl, shift) {
            (keys::LEFT | keys::UP, false, true) => {
                self.plan_edit(|p, s| p.shift(s, -1));
            }
            (keys::RIGHT | keys::DOWN, false, true) => {
                self.plan_edit(|p, s| p.shift(s, 1));
            }
            (keys::LEFT, false, false) => self.page_sel = to(-1),
            (keys::RIGHT, false, false) => self.page_sel = to(1),
            (keys::UP, false, false) => self.page_sel = to(-cols),
            (keys::DOWN, false, false) => self.page_sel = to(cols),
            (keys::PRIOR, _, _) => self.page_sel = to(-cols * 3),
            (keys::NEXT, _, _) => self.page_sel = to(cols * 3),
            (keys::HOME, _, _) => self.page_sel = 0,
            (keys::END, _, _) => self.page_sel = n - 1,
            (0x72, false, false) => self.plan_edit(|p, s| {
                p.turn(s, 1);
                Some(s)
            }),
            (0x52, false, _) => self.plan_edit(|p, s| {
                p.turn(s, 3);
                Some(s)
            }),
            (keys::DELETE, _, _) => {
                if n == 1 {
                    self.flash("The last page stays", false);
                } else {
                    self.plan_edit(|p, s| p.remove(s).then(|| s.min(p.sheets.len() - 1)));
                }
            }
            (0x6d, false, false) => {
                let dir = self
                    .pdf()
                    .and_then(|d| d.path.parent())
                    .map(|p| format!("{}/", p.display()))
                    .unwrap_or_default();
                self.ask_with(Ask::Merge, "Add the pages of this PDF at the end:", dir);
            }
            (0x65, false, false) => {
                let text = format!("{}", self.page_sel + 1);
                self.ask_with(
                    Ask::Extract,
                    "Save these pages as a new file (3, 2-5 or 1,4):",
                    text,
                );
            }
            (0x7a, true, _) => {
                let Some(d) = self.pdf_mut() else { return true };
                match d.plan_undo.pop() {
                    Some(p) => {
                        let len = p.sheets.len();
                        d.plan = Some(p);
                        self.page_sel = self.page_sel.min(len - 1);
                    }
                    None => self.flash("Nothing to undo", false),
                }
            }
            (keys::ESCAPE | keys::RETURN | keys::KP_ENTER, _, _) | (0x70, false, false) => {
                self.leave_pages()
            }
            _ => return false,
        }
        self.pages_follow();
        self.dirty = true;
        true
    }

    /// Append another PDF's pages (a path typed, or a PDF dropped).
    pub fn merge(&mut self, path: &Path) {
        let path = crate::files::expand_home(path);
        if !crate::picture::is_pdf(&path) || !path.is_file() {
            self.flash(&format!("{} is not a PDF", path.display()), true);
            return;
        }
        match crate::pdf::info(&path) {
            Ok(info) if info.encrypted => {
                self.flash(&format!("{} is encrypted", name_of(&path)), true)
            }
            Ok(info) => {
                let n = info.pages.len();
                let name = name_of(&path);
                self.plan_edit(|p, _| {
                    p.append(path, n);
                    Some(p.sheets.len() - n)
                });
                self.flash(&format!("{n} pages of {name} added at the end"), false);
            }
            Err(e) => self.flash(&format!("{}: {e}", name_of(&path)), true),
        }
    }

    /// Save the pages typed (numbers as shown in the pages view) as a file
    /// of their own.
    pub fn extract(&mut self, text: &str) {
        let Some(d) = self.pdf() else { return };
        let Some(plan) = d.plan.as_ref() else { return };
        let Some(range) = pdfpages::parse_range(text, plan.sheets.len()) else {
            self.flash(
                &format!("Write page numbers from 1 to {}", plan.sheets.len()),
                true,
            );
            return;
        };
        let sheets: Vec<pdfpages::Sheet> = range.iter().map(|&i| plan.sheets[i]).collect();
        let label = text.trim().replace([' ', ','], "_");
        let to = crate::pdf::copy_path(&d.path, &format!("pages-{label}"));
        match pdfpages::write(&plan.files, &sheets, &to) {
            Ok(()) => {
                let m = format!(
                    "{} page{} saved as {}",
                    sheets.len(),
                    if sheets.len() == 1 { "" } else { "s" },
                    name_of(&to)
                );
                if let Some(dir) = to.parent() {
                    let cur = self.current().cloned();
                    self.list = crate::files::list(dir, self.sort);
                    if let Some(c) = cur {
                        self.index = self.list.iter().position(|p| *p == c).unwrap_or(0);
                    }
                }
                self.flash(&m, false);
            }
            Err(e) => self.flash(&format!("Not saved: {e}"), true),
        }
    }

    /// Ctrl+S with page changes: the whole new order as `name-pages.pdf`,
    /// which is then shown.
    pub fn save_pages(&mut self) -> bool {
        let Some(d) = self.pdf() else { return false };
        let Some(plan) = d.plan.clone() else {
            return false;
        };
        let to = crate::pdf::copy_path(&d.path, "pages");
        match pdfpages::write(&plan.files, &plan.sheets, &to) {
            Ok(()) => {
                let name = name_of(&to);
                self.pages_view = false;
                if let Some(d) = self.pdf_mut() {
                    d.plan = None;
                }
                self.after_save(to);
                self.load_current();
                self.refit();
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

    pub fn strip_cell(&self) -> (i32, i32) {
        let th = self.ui.size(self.font_px).height;
        (172, 220 + th + 6)
    }

    pub fn strip_cols(&self) -> i32 {
        let (cw, _) = self.strip_cell();
        ((self.view().w - 16) / (cw + 14)).max(1)
    }

    fn strip_origin(&self) -> (i32, i32) {
        let (cw, _) = self.strip_cell();
        let v = self.view();
        let cols = self.strip_cols();
        (
            v.x + (v.w - cols * (cw + 14) + 14) / 2,
            v.y + 12 - self.strip_scroll,
        )
    }

    pub fn strip_hit(&self, x: i32, y: i32) -> Option<usize> {
        let (cw, ch) = self.strip_cell();
        let (left, top) = self.strip_origin();
        let (gx, gy) = (x - left, y - top);
        if gx < 0 || gy < 0 || gx % (cw + 14) >= cw {
            return None;
        }
        let (c, r) = (gx / (cw + 14), gy / (ch + 14));
        if c >= self.strip_cols() {
            return None;
        }
        let i = (r * self.strip_cols() + c) as usize;
        (i < self.plan_len()).then_some(i)
    }

    pub fn pages_click(&mut self, x: i32, y: i32) {
        if let Some(i) = self.strip_hit(x, y) {
            self.page_sel = i;
            self.dirty = true;
        }
    }

    fn pages_follow(&mut self) {
        let (_, ch) = self.strip_cell();
        let row = self.page_sel as i32 / self.strip_cols();
        let v = self.view();
        let top = row * (ch + 14);
        if top < self.strip_scroll {
            self.strip_scroll = top;
        } else if top + ch + 24 > self.strip_scroll + v.h {
            self.strip_scroll = top + ch + 24 - v.h;
        }
    }

    /// Ask for the thumbnails of the pages in sight.
    pub fn pages_tick(&mut self) {
        let (_, ch) = self.strip_cell();
        let v = self.view();
        let cols = self.strip_cols() as usize;
        let first = (self.strip_scroll / (ch + 14)).max(0) as usize * cols;
        let last = first + ((v.h / (ch + 14)) as usize + 2) * cols;
        let Some(d) = self.pdf() else { return };
        let Some(plan) = d.plan.as_ref() else { return };
        let want: Vec<(PathBuf, usize)> = plan
            .sheets
            .iter()
            .skip(first)
            .take(last - first)
            .map(|s| (plan.files[s.src].clone(), s.page))
            .filter(|k| !d.strip.contains_key(k))
            .collect();
        for (path, page) in want {
            self.jobs.ask(Job::Page {
                path,
                page,
                dpi: STRIP_DPI,
            });
        }
    }

    pub fn strip_done(&mut self, path: PathBuf, page: usize, raster: PageRaster) {
        if let Some(d) = self.pdf_mut() {
            if d.plan.as_ref().is_some_and(|p| p.files.contains(&path)) {
                d.strip.insert((path, page), raster);
            }
        }
    }

    // ------------------------------------------------------------ drawing

    /// Found text, the selection and the form fields on page `i`, drawn at
    /// (x, y) on screen.
    pub fn draw_pdf_marks(&mut self, i: usize, x: f64, y: f64) {
        let z = self.zoom;
        let to_rect = |b: &[f64; 4]| {
            Rect::new(
                (x + b[0] * z).floor() as i32,
                (y + b[1] * z).floor() as i32,
                ((b[2] - b[0]) * z).ceil().max(1.0) as i32 + 1,
                ((b[3] - b[1]) * z).ceil().max(1.0) as i32 + 1,
            )
        };
        let mark = self.pal.mark();
        if let Some(f) = &self.find {
            let rects: Vec<(Rect, bool)> = f
                .hits
                .iter()
                .enumerate()
                .filter(|(_, h)| h.page == i)
                .flat_map(|(k, h)| h.boxes.iter().map(move |b| (to_rect(b), k == f.cur)))
                .collect();
            for (r, cur) in rects {
                if cur {
                    self.cv.tint(r, 0xff8a00, 130);
                    self.cv.frame(r.inset(-1), 0xd05a00);
                } else {
                    self.cv.tint(r, 0xffd400, 110);
                }
            }
        }
        if let (Some((page, a, b)), Some(Doc::Pdf(d))) = (self.sel_range(), Some(&self.doc)) {
            if page == i {
                if let Some(ws) = d.words.as_ref().and_then(|w| w.get(page)) {
                    let rects: Vec<Rect> = ws[a..=b].iter().map(|w| to_rect(&w.b)).collect();
                    for r in rects {
                        self.cv.tint(r, 0x2f6fdf, 90);
                    }
                }
            }
        }
        if self.form {
            let Doc::Pdf(d) = &self.doc else { return };
            let Some(fields) = &d.fields else { return };
            let mut draws = Vec::new();
            for (k, f) in fields.iter().enumerate() {
                for w in f.widgets.iter().filter(|w| w.page == i) {
                    draws.push((to_rect(&w.rect), k, f.clone(), w.clone()));
                }
            }
            for (r, k, f, w) in draws {
                let sel = self.field_sel == Some(k);
                if f.changed {
                    self.cv.fill(r, 0xffffff);
                }
                self.cv
                    .tint(r, if f.read_only { 0x888888 } else { 0x2f6fdf }, 40);
                self.cv.frame(r, if sel { mark } else { 0x7a9ad0 });
                if sel {
                    self.cv.frame(r.inset(-1), mark);
                }
                if !f.changed {
                    continue;
                }
                match &f.kind {
                    Kind::Check | Kind::Radio => {
                        if f.is_on(&w) {
                            let s = r.inset((r.w.min(r.h) / 5).max(1));
                            let pts = if matches!(f.kind, Kind::Check) {
                                vec![
                                    (s.x as f64, (s.y + s.h / 2) as f64),
                                    ((s.x + s.w / 3) as f64, (s.y + s.h) as f64),
                                    ((s.x + s.w) as f64, s.y as f64),
                                ]
                            } else {
                                Vec::new()
                            };
                            if pts.is_empty() {
                                self.cv.fill(s, 0x111111);
                            } else {
                                self.cv.stroke(&pts, 0x111111, (z * 1.6).max(1.5));
                            }
                        }
                    }
                    _ => {
                        let size = if f.size > 0.0 {
                            f.size
                        } else {
                            ((w.rect[3] - w.rect[1]) * 0.7).clamp(4.0, 12.0)
                        };
                        let px = (size * z) as f32;
                        if px >= 3.0 {
                            let fs = self.sans.size(px);
                            let text = forms::shown_value(&f);
                            let line = text.lines().next().unwrap_or("").to_string();
                            let ty = if matches!(f.kind, Kind::Text { multiline: true }) {
                                r.y + 2
                            } else {
                                r.y + (r.h - fs.height) / 2
                            };
                            let line = self.sans.ellipsize(&line, fs, r.w - 4);
                            self.sans.draw(
                                &mut self.cv,
                                r.x + (2.0 * z) as i32,
                                ty,
                                &line,
                                fs,
                                0x111111,
                            );
                        }
                    }
                }
            }
        }
    }

    pub fn draw_pages(&mut self) {
        let p = self.pal;
        let v = self.view();
        let (cw, ch) = self.strip_cell();
        let cols = self.strip_cols();
        let (left, top) = self.strip_origin();
        let size = self.ui.size(self.font_px);
        self.cv.set_clip(v);
        let Doc::Pdf(d) = &self.doc else { return };
        let Some(plan) = d.plan.clone() else { return };
        let rows = (plan.sheets.len() as i32 + cols - 1) / cols;
        let max_scroll = (rows * (ch + 14) + 24 - v.h).max(0);
        if self.strip_scroll > max_scroll {
            self.strip_scroll = max_scroll;
            self.dirty = true;
        }
        let shadow = crate::theme::blend(p.background, 0x000000, 0.6);
        for (i, s) in plan.sheets.iter().enumerate() {
            let (c, r) = (i as i32 % cols, i as i32 / cols);
            let x = left + c * (cw + 14);
            let y = top + r * (ch + 14);
            if y + ch < v.y || y > v.y + v.h {
                continue;
            }
            let cell = Rect::new(x, y, cw, ch);
            if i == self.page_sel {
                self.cv.fill(cell.inset(-4), p.selected);
                self.cv.frame(cell.inset(-4), p.mark());
            }
            let boxh = ch - size.height - 6;
            let key = (plan.files[s.src].clone(), s.page);
            let Doc::Pdf(d) = &self.doc else { return };
            match d.strip.get(&key) {
                Some(ras) => {
                    let (buf, w, h) = turn_px(&ras.px, ras.w as usize, ras.h as usize, s.turn);
                    let f = (cw as f64 / w as f64).min(boxh as f64 / h as f64).min(1.0);
                    let (tw, th) = (
                        ((w as f64 * f) as i32).max(1),
                        ((h as f64 * f) as i32).max(1),
                    );
                    let mut out = Vec::with_capacity((tw * th) as usize);
                    for yy in 0..th {
                        let sy = ((yy as f64 + 0.5) / f) as usize;
                        for xx in 0..tw {
                            let sx = ((xx as f64 + 0.5) / f) as usize;
                            out.push(buf[sy.min(h - 1) * w + sx.min(w - 1)]);
                        }
                    }
                    let (px, py) = (x + (cw - tw) / 2, y + (boxh - th) / 2);
                    self.cv.fill(Rect::new(px + 3, py + 3, tw, th), shadow);
                    self.cv
                        .picture(px, py, tw, th, &out, tw as usize, (0xffffff, 0xffffff));
                }
                None => {
                    let r = Rect::new(x + cw / 6, y + 4, cw * 2 / 3, boxh - 8);
                    self.cv.fill(r, 0xf4f4f4);
                }
            }
            let mut label = format!("{}", i + 1);
            if s.src > 0 {
                label.push_str(&format!(
                    "  {} p{}",
                    name_of(&plan.files[s.src]),
                    s.page + 1
                ));
            } else if s.page != i {
                label.push_str(&format!("  (was {})", s.page + 1));
            }
            if s.turn != 0 {
                label.push_str(&format!("  {}°", s.turn * 90));
            }
            let label = self.ui.ellipsize(&label, size, cw);
            let lw = self.ui.width(&label, size);
            let color = if i == self.page_sel {
                p.foreground
            } else {
                p.muted
            };
            self.ui.draw(
                &mut self.cv,
                x + (cw - lw) / 2,
                y + boxh + 4,
                &label,
                size,
                color,
            );
        }
        self.cv.reset_clip();
    }
}

/// Pixels turned by quarter turns (clockwise), with the new width and height.
pub fn turn_px(px: &[u32], w: usize, h: usize, turn: u32) -> (Vec<u32>, usize, usize) {
    match turn % 4 {
        0 => (px.to_vec(), w, h),
        2 => (px.iter().rev().copied().collect(), w, h),
        q => {
            let mut out = vec![0u32; w * h];
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = if q == 1 {
                        (h - 1 - y, x)
                    } else {
                        (y, w - 1 - x)
                    };
                    out[ny * h + nx] = px[y * w + x];
                }
            }
            (out, h, w)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_turn_clockwise() {
        // A row of two turned right is a column, the first on top.
        let (p, w, h) = turn_px(&[1, 2], 2, 1, 1);
        assert_eq!((w, h), (1, 2));
        assert_eq!(p, vec![1, 2]);
        let (p, w, h) = turn_px(&[1, 2, 3, 4], 2, 2, 1);
        assert_eq!((w, h), (2, 2));
        // 1 2 / 3 4 turned right is 3 1 / 4 2.
        assert_eq!(p, vec![3, 1, 4, 2]);
        assert_eq!(turn_px(&[1, 2, 3, 4], 2, 2, 3).0, vec![2, 4, 1, 3]);
        assert_eq!(turn_px(&[1, 2, 3, 4], 2, 2, 2).0, vec![4, 3, 2, 1]);
    }
}
