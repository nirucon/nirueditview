//! Pages of a PDF put in a new order: turned, removed, moved, other PDFs
//! added at the end, or a range of them saved as a file of its own. The
//! result is written as a new file with lopdf (not an incremental update,
//! so a removed page is really gone from it); the original stays as it is.
use lopdf::{dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// One page of the result: which file it comes from, its page there
/// (0-based) and the quarter turns added to it (clockwise).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub src: usize,
    pub page: usize,
    pub turn: u32,
}

/// The page list being edited: the files, and the pages in their new order.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub files: Vec<PathBuf>,
    pub sheets: Vec<Sheet>,
}

impl Plan {
    pub fn new(path: PathBuf, pages: usize) -> Self {
        Self {
            files: vec![path],
            sheets: (0..pages)
                .map(|page| Sheet {
                    src: 0,
                    page,
                    turn: 0,
                })
                .collect(),
        }
    }

    /// Nothing changed: the same file, every page in order, none turned.
    pub fn untouched(&self, pages: usize) -> bool {
        self.files.len() == 1
            && self.sheets.len() == pages
            && self
                .sheets
                .iter()
                .enumerate()
                .all(|(i, s)| s.src == 0 && s.page == i && s.turn == 0)
    }

    pub fn turn(&mut self, i: usize, quarter: u32) {
        if let Some(s) = self.sheets.get_mut(i) {
            s.turn = (s.turn + quarter) % 4;
        }
    }

    /// Move page `i` by `d` places; the new place, or None at an end.
    pub fn shift(&mut self, i: usize, d: i64) -> Option<usize> {
        let j = i as i64 + d;
        if i >= self.sheets.len() || j < 0 || j >= self.sheets.len() as i64 {
            return None;
        }
        let s = self.sheets.remove(i);
        self.sheets.insert(j as usize, s);
        Some(j as usize)
    }

    pub fn remove(&mut self, i: usize) -> bool {
        if self.sheets.len() > 1 && i < self.sheets.len() {
            self.sheets.remove(i);
            true
        } else {
            false
        }
    }

    pub fn append(&mut self, path: PathBuf, pages: usize) {
        let src = self.files.len();
        self.files.push(path);
        self.sheets
            .extend((0..pages).map(|page| Sheet { src, page, turn: 0 }));
    }
}

/// "3", "2-5", "1,3,7-9" (1-based, as the pages are numbered on screen)
/// into 0-based page numbers, in the order written.
pub fn parse_range(text: &str, count: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>().ok()?, b.trim().parse().ok()?),
            None => {
                let n = part.parse::<usize>().ok()?;
                (n, n)
            }
        };
        if a == 0 || b == 0 || a > count || b > count || a > b {
            return None;
        }
        out.extend(a - 1..b);
    }
    (!out.is_empty()).then_some(out)
}

/// Keys a page takes from its parents in the page tree when it has none.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut id = page;
    for _ in 0..64 {
        let d = doc.get_dictionary(id).ok()?;
        if let Ok(v) = d.get(key) {
            return Some(v.clone());
        }
        id = d.get(b"Parent").and_then(Object::as_reference).ok()?;
    }
    None
}

fn shift_refs(o: &mut Object, by: u32) {
    match o {
        Object::Reference(id) => id.0 += by,
        Object::Array(a) => a.iter_mut().for_each(|o| shift_refs(o, by)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, o)| shift_refs(o, by)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, o)| shift_refs(o, by)),
        _ => {}
    }
}

fn null_refs(o: &mut Object, gone: &HashSet<ObjectId>) {
    match o {
        Object::Reference(id) if gone.contains(id) => *o = Object::Null,
        Object::Array(a) => a.iter_mut().for_each(|o| null_refs(o, gone)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, o)| null_refs(o, gone)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, o)| null_refs(o, gone)),
        _ => {}
    }
}

fn load(path: &Path) -> Result<Document, String> {
    let doc = Document::load(path)
        .map_err(|e| format!("cannot read {}: {e}", crate::app::name_of(path)))?;
    if doc.is_encrypted() {
        return Err(format!(
            "{} is encrypted; it cannot be changed here",
            crate::app::name_of(path)
        ));
    }
    Ok(doc)
}

/// Write the pages `sheets` names (from `files`) as a new PDF at `dst`.
pub fn write(files: &[PathBuf], sheets: &[Sheet], dst: &Path) -> Result<(), String> {
    if sheets.is_empty() {
        return Err("no pages".into());
    }
    let mut doc = load(&files[0])?;
    // Every file's pages, by file and page number.
    let mut ids: Vec<BTreeMap<u32, ObjectId>> = vec![doc.get_pages()];
    for f in &files[1..] {
        let mut other = load(f)?;
        let by = doc.max_id;
        let pages: BTreeMap<u32, ObjectId> = other
            .get_pages()
            .into_iter()
            .map(|(n, id)| (n, (id.0 + by, id.1)))
            .collect();
        let objects = std::mem::take(&mut other.objects);
        for ((n, g), mut o) in objects {
            shift_refs(&mut o, by);
            doc.objects.insert((n + by, g), o);
        }
        doc.max_id = doc.max_id.max(other.max_id + by);
        ids.push(pages);
    }
    // Each page used once: a page taken twice is copied.
    let mut used: HashSet<ObjectId> = HashSet::new();
    let mut kids = Vec::new();
    let root = doc.new_object_id();
    for s in sheets {
        let Some(&id) = ids.get(s.src).and_then(|m| m.get(&(s.page as u32 + 1))) else {
            return Err(format!("page {} is missing", s.page + 1));
        };
        let mut dict = doc.get_dictionary(id).map_err(|e| e.to_string())?.clone();
        for key in INHERITED {
            if !dict.has(key) {
                if let Some(v) = inherited(&doc, id, key) {
                    dict.set(key, v);
                }
            }
        }
        let rot = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        dict.set(
            "Rotate",
            Object::Integer((rot + 90 * s.turn as i64).rem_euclid(360)),
        );
        dict.set("Parent", Object::Reference(root));
        let target = if used.insert(id) {
            doc.objects.insert(id, Object::Dictionary(dict));
            id
        } else {
            // Annotations belong to one page; a copy goes without them.
            dict.remove(b"Annots");
            doc.add_object(Object::Dictionary(dict))
        };
        kids.push(Object::Reference(target));
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        root,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    // Pages of the first file left out: whatever pointed at them (an
    // outline entry, a link, a form field's widget) now points nowhere, so
    // they are not kept alive and leave the file.
    let gone: HashSet<ObjectId> = ids[0]
        .values()
        .filter(|id| !used.contains(id))
        .copied()
        .collect();
    if !gone.is_empty() {
        for o in doc.objects.values_mut() {
            null_refs(o, &gone);
        }
        for id in &gone {
            doc.objects.remove(id);
        }
    }
    doc.catalog_mut()
        .map_err(|e| e.to_string())?
        .set("Pages", Object::Reference(root));
    doc.prune_objects();
    let tmp = dst.with_extension("pdf.part");
    doc.save(&tmp)
        .map_err(|e| format!("cannot write {}: {e}", crate::app::name_of(dst)))?;
    std::fs::rename(&tmp, dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", crate::app::name_of(dst))
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use lopdf::Stream;

    /// A PDF of `n` pages, each with its number as text, under a page tree
    /// of two levels that gives them their size and resources.
    pub fn make(path: &Path, label: &str, n: usize) {
        let mut doc = Document::with_version("1.5");
        let top = doc.new_object_id();
        let mid = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let mut kids = Vec::new();
        for i in 0..n {
            let c = doc.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 30 Tf 72 700 Td ({label} {}) Tj ET", i + 1).into_bytes(),
            ));
            kids.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => mid, "Contents" => c,
            })));
        }
        doc.objects.insert(
            mid,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Parent" => top, "Kids" => kids, "Count" => n as i64,
            }),
        );
        doc.objects.insert(
            top,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(mid)],
                "Count" => n as i64,
                "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => top });
        doc.trailer.set("Root", cat);
        doc.save(path).unwrap();
    }

    fn texts(path: &Path) -> Vec<String> {
        let doc = Document::load(path).unwrap();
        doc.get_pages()
            .values()
            .map(|id| String::from_utf8_lossy(&doc.get_page_content(*id)).into_owned())
            .collect()
    }

    #[test]
    fn plans_turn_move_remove_and_append() {
        let mut p = Plan::new("a.pdf".into(), 3);
        assert!(p.untouched(3));
        p.turn(1, 1);
        assert!(!p.untouched(3));
        p.turn(1, 3);
        assert!(p.untouched(3));
        assert_eq!(p.shift(0, 1), Some(1));
        assert_eq!(p.sheets[1].page, 0);
        assert_eq!(p.shift(2, 1), None);
        assert!(p.remove(0));
        p.append("b.pdf".into(), 2);
        assert_eq!(p.sheets.len(), 4);
        assert_eq!(
            p.sheets[3],
            Sheet {
                src: 1,
                page: 1,
                turn: 0
            }
        );
        let mut one = Plan::new("a.pdf".into(), 1);
        assert!(!one.remove(0), "the last page stays");
    }

    #[test]
    fn ranges_are_read_one_based() {
        assert_eq!(parse_range("2-4", 5), Some(vec![1, 2, 3]));
        assert_eq!(parse_range("1, 3,5-5", 5), Some(vec![0, 2, 4]));
        assert_eq!(parse_range("0", 5), None);
        assert_eq!(parse_range("4-2", 5), None);
        assert_eq!(parse_range("6", 5), None);
        assert_eq!(parse_range("x", 5), None);
    }

    #[test]
    fn pages_are_reordered_turned_removed_and_merged() {
        let dir = std::env::temp_dir().join(format!("nev-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        make(&a, "Alpha", 3);
        make(&b, "Beta", 2);
        let before = std::fs::read(&a).unwrap();
        let mut plan = Plan::new(a.clone(), 3);
        plan.remove(1); // Alpha 2 goes
        plan.shift(1, -1); // Alpha 3 first
        plan.turn(0, 1);
        plan.append(b.clone(), 2);
        let out = dir.join("out.pdf");
        write(&plan.files, &plan.sheets, &out).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), before, "the original is kept");
        let t = texts(&out);
        assert_eq!(t.len(), 4);
        assert!(
            t[0].contains("Alpha 3") && t[1].contains("Alpha 1"),
            "{t:?}"
        );
        assert!(t[2].contains("Beta 1") && t[3].contains("Beta 2"), "{t:?}");
        let raw = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
        assert!(!raw.contains("Alpha 2"), "a removed page leaves the file");
        let doc = Document::load(&out).unwrap();
        let first = doc.get_dictionary(doc.get_pages()[&1]).unwrap();
        assert_eq!(first.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
        // Inherited size and fonts came along with each page.
        assert!(first.get(b"MediaBox").is_ok() && first.get(b"Resources").is_ok());
        // A range saved on its own.
        let part = dir.join("part.pdf");
        let range: Vec<Sheet> = parse_range("2-3", 3)
            .unwrap()
            .into_iter()
            .map(|page| Sheet {
                src: 0,
                page,
                turn: 0,
            })
            .collect();
        write(std::slice::from_ref(&a), &range, &part).unwrap();
        let t = texts(&part);
        assert_eq!(t.len(), 2);
        assert!(t[0].contains("Alpha 2") && t[1].contains("Alpha 3"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
