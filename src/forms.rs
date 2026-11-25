//! PDF forms (AcroForm): the fields a PDF asks to be filled in, read with
//! lopdf when you start filling them in (E), and the values written back
//! in the same incremental update as the stamps, each widget with an
//! appearance of its own so every viewer (Poppler too) shows the value.
use crate::pdf::Page;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text {
        multiline: bool,
    },
    Check,
    Radio,
    /// A list or a drop-down: (value written, text shown) for each option;
    /// `edit` when other text may be typed too.
    Choice {
        options: Vec<(String, String)>,
        edit: bool,
    },
}

/// Where a field is on a page, in shown points (left, top, right, bottom).
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub id: ObjectId,
    pub page: usize,
    pub rect: [f64; 4],
    /// The rectangle in PDF user space (x0, y0, x1, y1), for writing.
    pub pdf_rect: [f64; 4],
    /// The name of the "on" state of a check box or a radio button.
    pub on: Option<String>,
    /// The widget is turned (`/MK /R`), in degrees.
    pub turn: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: ObjectId,
    pub name: String,
    pub kind: Kind,
    /// Text, the option's value, or the state of a button ("Off" or its
    /// on state).
    pub value: String,
    pub widgets: Vec<Widget>,
    pub read_only: bool,
    /// Font size from the field's default appearance; 0 is "fit".
    pub size: f64,
    pub changed: bool,
}

impl Field {
    pub fn is_on(&self, w: &Widget) -> bool {
        w.on.as_deref().is_some_and(|on| on == self.value)
    }
}

const READ_ONLY: i64 = 1;
const MULTILINE: i64 = 1 << 12;
const RADIO: i64 = 1 << 15;
const PUSH: i64 = 1 << 16;
const EDIT: i64 = 1 << 18;

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

fn dict_of(doc: &Document, id: ObjectId) -> Option<&Dictionary> {
    doc.get_dictionary(id).ok()
}

fn text(doc: &Document, o: &Object) -> String {
    match deref(doc, o) {
        Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
        o @ Object::String(..) => lopdf::decode_text_string(o).unwrap_or_default(),
        Object::Array(a) => a.first().map(|o| text(doc, o)).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The size in "/Helv 12 Tf 0 g".
pub fn da_size(da: &str) -> f64 {
    let t: Vec<&str> = da.split_whitespace().collect();
    t.iter()
        .position(|w| *w == "Tf")
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| t[i].parse().ok())
        .unwrap_or(0.0)
}

#[derive(Clone, Default)]
struct Inherit {
    name: String,
    ft: Vec<u8>,
    ff: i64,
    v: Option<Object>,
    da: String,
    opt: Option<Object>,
}

/// Every field that can be filled in, with its widgets on the pages.
pub fn read(path: &Path, pages: &[Page]) -> Result<Vec<Field>, String> {
    let doc = Document::load(path).map_err(|e| format!("cannot read the PDF: {e}"))?;
    if doc.is_encrypted() {
        return Err("the PDF is encrypted; its form cannot be filled in here".into());
    }
    let page_ids = doc.get_pages();
    let mut page_of: HashMap<ObjectId, usize> = HashMap::new();
    let mut annot_page: HashMap<ObjectId, usize> = HashMap::new();
    for (n, id) in &page_ids {
        let i = *n as usize - 1;
        page_of.insert(*id, i);
        if let Some(Object::Array(a)) = dict_of(&doc, *id)
            .and_then(|d| d.get(b"Annots").ok())
            .map(|o| deref(&doc, o))
        {
            for r in a {
                if let Ok(rid) = r.as_reference() {
                    annot_page.insert(rid, i);
                }
            }
        }
    }
    let Some(Object::Array(roots)) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .map(|o| deref(&doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|f| f.get(b"Fields").ok())
        .map(|o| deref(&doc, o))
    else {
        return Ok(Vec::new());
    };
    let ctx = Ctx {
        doc: &doc,
        pages,
        page_of: &page_of,
        annot_page: &annot_page,
    };
    let mut out = Vec::new();
    for r in roots {
        if let Ok(id) = r.as_reference() {
            ctx.walk(id, &Inherit::default(), &mut out, 0);
        }
    }
    Ok(out)
}

struct Ctx<'a> {
    doc: &'a Document,
    pages: &'a [Page],
    page_of: &'a HashMap<ObjectId, usize>,
    annot_page: &'a HashMap<ObjectId, usize>,
}

impl Ctx<'_> {
    fn walk(&self, id: ObjectId, up: &Inherit, out: &mut Vec<Field>, depth: usize) {
        let Some(d) = dict_of(self.doc, id) else {
            return;
        };
        if depth > 32 {
            return;
        }
        let mut me = up.clone();
        if let Ok(t) = d.get(b"T") {
            let t = text(self.doc, t);
            me.name = if up.name.is_empty() {
                t
            } else {
                format!("{}.{t}", up.name)
            };
        }
        if let Ok(Object::Name(n)) = d.get(b"FT").map(|o| deref(self.doc, o)) {
            me.ft = n.clone();
        }
        if let Ok(f) = d.get(b"Ff").and_then(|o| deref(self.doc, o).as_i64()) {
            me.ff = f;
        }
        if let Ok(v) = d.get(b"V") {
            me.v = Some(deref(self.doc, v).clone());
        }
        if let Ok(da) = d
            .get(b"DA")
            .map(|o| deref(self.doc, o))
            .and_then(Object::as_str)
        {
            me.da = String::from_utf8_lossy(da).into_owned();
        }
        if let Ok(o) = d.get(b"Opt") {
            me.opt = Some(deref(self.doc, o).clone());
        }
        let kids: Vec<ObjectId> = match d.get(b"Kids").map(|o| deref(self.doc, o)) {
            Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        };
        let sub: Vec<ObjectId> = kids
            .iter()
            .copied()
            .filter(|k| dict_of(self.doc, *k).is_some_and(|d| d.has(b"T")))
            .collect();
        if !sub.is_empty() {
            for k in sub {
                self.walk(k, &me, out, depth + 1);
            }
            return;
        }
        let widget_ids: Vec<ObjectId> = if kids.is_empty() { vec![id] } else { kids };
        let kind = match me.ft.as_slice() {
            b"Tx" => Kind::Text {
                multiline: me.ff & MULTILINE != 0,
            },
            b"Btn" if me.ff & PUSH != 0 => return,
            b"Btn" if me.ff & RADIO != 0 => Kind::Radio,
            b"Btn" => Kind::Check,
            b"Ch" => Kind::Choice {
                options: self.options(me.opt.as_ref()),
                edit: me.ff & EDIT != 0,
            },
            _ => return,
        };
        let widgets: Vec<Widget> = widget_ids.iter().filter_map(|w| self.widget(*w)).collect();
        if widgets.is_empty() {
            return;
        }
        let value = match (&kind, &me.v) {
            (_, None) if matches!(kind, Kind::Check | Kind::Radio) => {
                // No value: what the widgets show.
                widget_ids
                    .iter()
                    .filter_map(|w| dict_of(self.doc, *w))
                    .filter_map(|d| d.get(b"AS").ok().map(|o| text(self.doc, o)))
                    .find(|s| s != "Off")
                    .unwrap_or_else(|| "Off".into())
            }
            (_, Some(v)) => text(self.doc, v),
            _ => String::new(),
        };
        out.push(Field {
            id,
            name: me.name.clone(),
            kind,
            value,
            widgets,
            read_only: me.ff & READ_ONLY != 0,
            size: da_size(&me.da),
            changed: false,
        });
    }

    fn options(&self, o: Option<&Object>) -> Vec<(String, String)> {
        let Some(Object::Array(a)) = o else {
            return Vec::new();
        };
        a.iter()
            .map(|o| match deref(self.doc, o) {
                Object::Array(p) if p.len() >= 2 => (text(self.doc, &p[0]), text(self.doc, &p[1])),
                o => {
                    let t = text(self.doc, o);
                    (t.clone(), t)
                }
            })
            .collect()
    }

    fn widget(&self, id: ObjectId) -> Option<Widget> {
        let d = dict_of(self.doc, id)?;
        let page = d
            .get(b"P")
            .ok()
            .and_then(|p| p.as_reference().ok())
            .and_then(|p| self.page_of.get(&p).copied())
            .or_else(|| self.annot_page.get(&id).copied())?;
        let r: Vec<f64> = match d.get(b"Rect").map(|o| deref(self.doc, o)) {
            Ok(Object::Array(a)) => a
                .iter()
                .filter_map(|o| deref(self.doc, o).as_float().ok().map(f64::from))
                .collect(),
            _ => return None,
        };
        if r.len() != 4 {
            return None;
        }
        let pdf_rect = [
            r[0].min(r[2]),
            r[1].min(r[3]),
            r[0].max(r[2]),
            r[1].max(r[3]),
        ];
        let p = self.pages.get(page)?;
        let a = p.shown_of(pdf_rect[0], pdf_rect[1]);
        let b = p.shown_of(pdf_rect[2], pdf_rect[3]);
        let on = d
            .get(b"AP")
            .ok()
            .map(|o| deref(self.doc, o))
            .and_then(|o| o.as_dict().ok())
            .and_then(|ap| ap.get(b"N").ok())
            .map(|o| deref(self.doc, o))
            .and_then(|o| o.as_dict().ok())
            .and_then(|n| {
                n.iter()
                    .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                    .find(|k| k != "Off")
            });
        let turn = d
            .get(b"MK")
            .ok()
            .map(|o| deref(self.doc, o))
            .and_then(|o| o.as_dict().ok())
            .and_then(|mk| mk.get(b"R").ok())
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
            .rem_euclid(360);
        Some(Widget {
            id,
            page,
            rect: [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)],
            pdf_rect,
            on,
            turn,
        })
    }
}

/// The value a choice takes from what was typed: an option's number, its
/// text or its value (any case), or the text itself when typing is allowed.
pub fn choose(kind: &Kind, typed: &str) -> Option<String> {
    let Kind::Choice { options, edit } = kind else {
        return Some(typed.to_string());
    };
    let t = typed.trim();
    if let Ok(n) = t.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return Some(options[n - 1].0.clone());
        }
    }
    let low = t.to_lowercase();
    options
        .iter()
        .find(|(v, s)| v.to_lowercase() == low || s.to_lowercase() == low)
        .map(|(v, _)| v.clone())
        .or_else(|| (*edit || t.is_empty()).then(|| t.to_string()))
}

/// The text a choice shows for its value.
pub fn shown_value(f: &Field) -> String {
    match &f.kind {
        Kind::Choice { options, .. } => options
            .iter()
            .find(|(v, _)| *v == f.value)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| f.value.clone()),
        _ => f.value.clone(),
    }
}

fn num(x: f64) -> String {
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The appearance of a text or choice widget: the value in Helvetica, in
/// the widget's own box (turned with it).
pub fn appearance(w: &Widget, value: &str, size: f64, multiline: bool) -> (Dictionary, Vec<u8>) {
    let (rw, rh) = (w.pdf_rect[2] - w.pdf_rect[0], w.pdf_rect[3] - w.pdf_rect[1]);
    let (bw, bh) = if w.turn % 180 == 90 {
        (rh, rw)
    } else {
        (rw, rh)
    };
    let lines: Vec<&str> = if multiline {
        value.lines().collect()
    } else {
        vec![value.lines().next().unwrap_or("")]
    };
    let mut fs = if size > 0.0 {
        size
    } else {
        (bh * 0.7).clamp(4.0, 12.0)
    };
    if !multiline {
        // Fit the width, the way viewers do with an automatic size.
        let w = crate::pdf::helvetica_width(lines[0], fs);
        if size <= 0.0 && w > bw - 4.0 && w > 0.0 {
            fs = (fs * (bw - 4.0) / w).max(4.0);
        }
    }
    let mut s = format!(
        "/Tx BMC q 1 1 {} {} re W n BT 0 g /NevHelv {} Tf ",
        num(bw - 2.0),
        num(bh - 2.0),
        num(fs)
    );
    if multiline {
        s.push_str(&format!(
            "{} TL 2 {} Td ",
            num(fs * 1.15),
            num(bh - 2.0 - fs)
        ));
    } else {
        s.push_str(&format!("2 {} Td ", num((bh - fs * 0.78) / 2.0)));
    }
    let mut body = s.into_bytes();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            body.extend_from_slice(b"T* ");
        }
        body.push(b'(');
        body.extend_from_slice(&crate::pdf::winansi(l));
        body.extend_from_slice(b") Tj ");
    }
    body.extend_from_slice(b"ET Q EMC");
    let matrix: Vec<Object> = match w.turn {
        90 => vec![
            0.into(),
            1.into(),
            (-1).into(),
            0.into(),
            0.into(),
            0.into(),
        ],
        180 => vec![
            (-1).into(),
            0.into(),
            0.into(),
            (-1).into(),
            0.into(),
            0.into(),
        ],
        270 => vec![
            0.into(),
            (-1).into(),
            1.into(),
            0.into(),
            0.into(),
            0.into(),
        ],
        _ => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
    };
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![0.into(), 0.into(), Object::Real(bw as f32), Object::Real(bh as f32)],
        "Matrix" => matrix,
    };
    (dict, body)
}

/// Write the changed fields into the incremental update `doc`; `font` is a
/// Helvetica (WinAnsi) font object in it.
pub fn write(
    doc: &mut lopdf::IncrementalDocument,
    fields: &[Field],
    font: ObjectId,
) -> Result<(), String> {
    let mut wide = false;
    for f in fields.iter().filter(|f| f.changed) {
        doc.opt_clone_object_to_new_document(f.id)
            .map_err(|e| e.to_string())?;
        let v = match f.kind {
            Kind::Check | Kind::Radio => Object::Name(f.value.clone().into_bytes()),
            _ => lopdf::text_string(&f.value),
        };
        wide |= f
            .value
            .chars()
            .any(|c| c != '?' && crate::pdf::winansi(&c.to_string()) == b"?");
        doc.new_document
            .get_object_mut(f.id)
            .and_then(Object::as_dict_mut)
            .map_err(|e| e.to_string())?
            .set("V", v);
        for w in &f.widgets {
            let ap = match &f.kind {
                Kind::Text { multiline } => Some(appearance(w, &f.value, f.size, *multiline)),
                Kind::Choice { .. } => Some(appearance(w, &shown_value(f), f.size, false)),
                _ => None,
            };
            let ap = ap.map(|(mut dict, body)| {
                dict.set(
                    "Resources",
                    dictionary! { "Font" => dictionary! { "NevHelv" => font } },
                );
                doc.new_document.add_object(Stream::new(dict, body))
            });
            doc.opt_clone_object_to_new_document(w.id)
                .map_err(|e| e.to_string())?;
            let wd = doc
                .new_document
                .get_object_mut(w.id)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?;
            match f.kind {
                Kind::Check | Kind::Radio => {
                    let state = if f.is_on(w) {
                        f.value.clone()
                    } else {
                        "Off".into()
                    };
                    wd.set("AS", Object::Name(state.into_bytes()));
                }
                _ => {
                    if let Some(ap) = ap {
                        wd.set("AP", dictionary! { "N" => ap });
                    }
                }
            }
        }
    }
    if wide {
        // Letters Helvetica cannot show here: let the viewer draw them.
        let cat_id = doc
            .get_prev_documents()
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|e| e.to_string())?;
        let form = doc
            .get_prev_documents()
            .catalog()
            .and_then(|c| c.get(b"AcroForm"))
            .map_err(|e| e.to_string())?
            .clone();
        match form {
            Object::Reference(fid) => {
                doc.opt_clone_object_to_new_document(fid)
                    .map_err(|e| e.to_string())?;
                doc.new_document
                    .get_object_mut(fid)
                    .and_then(Object::as_dict_mut)
                    .map_err(|e| e.to_string())?
                    .set("NeedAppearances", true);
            }
            Object::Dictionary(mut d) => {
                d.set("NeedAppearances", true);
                doc.opt_clone_object_to_new_document(cat_id)
                    .map_err(|e| e.to_string())?;
                doc.new_document
                    .get_object_mut(cat_id)
                    .and_then(Object::as_dict_mut)
                    .map_err(|e| e.to_string())?
                    .set("AcroForm", d);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A one-page form: a text field "name", a check box "agree", a
    /// drop-down "colour" and a radio group "size" with two buttons.
    pub fn make(path: &Path) {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let page = doc.new_object_id();
        let on = doc.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),12.into(),12.into()]}, b"0 g 2 2 8 8 re f".to_vec()));
        let off = doc.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),12.into(),12.into()]}, Vec::new()));
        let r = |a: i64, b: i64, c: i64, d: i64| -> Object {
            vec![a.into(), b.into(), c.into(), d.into()].into()
        };
        let name = doc.add_object(dictionary! {
            "FT" => "Tx", "T" => Object::string_literal("name"), "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => r(100, 700, 300, 720), "P" => page, "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        });
        let agree = doc.add_object(dictionary! {
            "FT" => "Btn", "T" => Object::string_literal("agree"), "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => r(100, 650, 112, 662), "P" => page, "V" => "Off", "AS" => "Off",
            "AP" => dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
        });
        let colour = doc.add_object(dictionary! {
            "FT" => "Ch", "T" => Object::string_literal("colour"), "Ff" => 1i64 << 17,
            "Type" => "Annot", "Subtype" => "Widget", "Rect" => r(100, 600, 250, 620), "P" => page,
            "Opt" => vec![Object::string_literal("Red"), vec![Object::string_literal("g"), Object::string_literal("Green")].into()],
        });
        let size = doc.new_object_id();
        let small = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => size, "Rect" => r(100, 550, 112, 562), "P" => page,
            "AS" => "S", "AP" => dictionary! { "N" => dictionary! { "S" => on, "Off" => off } },
        });
        let large = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => size, "Rect" => r(150, 550, 162, 562), "P" => page,
            "AS" => "Off", "AP" => dictionary! { "N" => dictionary! { "L" => on, "Off" => off } },
        });
        doc.objects.insert(
            size,
            Object::Dictionary(dictionary! {
                "FT" => "Btn", "T" => Object::string_literal("size"), "Ff" => RADIO, "V" => "S",
                "Kids" => vec![small.into(), large.into()],
            }),
        );
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 18 Tf 100 750 Td (Form) Tj ET".to_vec(),
        ));
        let font =
            doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        doc.objects.insert(page, Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages, "MediaBox" => r(0, 0, 595, 842), "Contents" => content,
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
            "Annots" => vec![name.into(), agree.into(), colour.into(), small.into(), large.into()],
        }));
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}),
        );
        let form = doc.add_object(dictionary! {
            "Fields" => vec![name.into(), agree.into(), colour.into(), size.into()],
            "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        });
        let cat = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages,"AcroForm"=>form});
        doc.trailer.set("Root", cat);
        doc.save(path).unwrap();
    }

    fn a4() -> Vec<Page> {
        vec![Page {
            x0: 0.0,
            y0: 0.0,
            x1: 595.0,
            y1: 842.0,
            rot: 0,
        }]
    }

    #[test]
    fn fields_are_read_with_kinds_values_and_places() {
        let dir = std::env::temp_dir().join(format!("nev-form-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.pdf");
        make(&p);
        let f = read(&p, &a4()).unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].name, "name");
        assert_eq!(f[0].kind, Kind::Text { multiline: false });
        // 100..300 across, 122..142 from the top of an 842-point page.
        assert_eq!(f[0].widgets[0].rect, [100.0, 122.0, 300.0, 142.0]);
        assert_eq!(f[1].kind, Kind::Check);
        assert_eq!(f[1].value, "Off");
        assert_eq!(f[1].widgets[0].on.as_deref(), Some("Yes"));
        let Kind::Choice { options, edit } = &f[2].kind else {
            panic!()
        };
        assert_eq!(options[1], ("g".to_string(), "Green".to_string()));
        assert!(!edit);
        assert_eq!(f[3].kind, Kind::Radio);
        assert_eq!(f[3].widgets.len(), 2);
        assert_eq!(f[3].value, "S");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn choices_take_a_number_a_text_or_a_value() {
        let k = Kind::Choice {
            options: vec![("r".into(), "Red".into()), ("g".into(), "Green".into())],
            edit: false,
        };
        assert_eq!(choose(&k, "2").as_deref(), Some("g"));
        assert_eq!(choose(&k, "red").as_deref(), Some("r"));
        assert_eq!(choose(&k, "G").as_deref(), Some("g"));
        assert_eq!(choose(&k, "Blue"), None);
        assert_eq!(da_size("/Helv 9.5 Tf 0 g"), 9.5);
        assert_eq!(da_size("0 g"), 0.0);
    }

    #[test]
    fn filled_values_are_written_with_appearances() {
        let dir = std::env::temp_dir().join(format!("nev-fill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.pdf");
        make(&p);
        let original = std::fs::read(&p).unwrap();
        let mut f = read(&p, &a4()).unwrap();
        f[0].value = "Ada Lovelace".into();
        f[1].value = "Yes".into();
        f[2].value = "g".into();
        f[3].value = "L".into();
        for x in f.iter_mut() {
            x.changed = true;
        }
        let out = dir.join("filled.pdf");
        crate::pdf::write_stamps(&p, &out, &a4(), &[], &f).unwrap();
        let written = std::fs::read(&out).unwrap();
        assert_eq!(
            &written[..original.len()],
            &original[..],
            "an incremental update"
        );
        let back = read(&out, &a4()).unwrap();
        assert_eq!(back[0].value, "Ada Lovelace");
        assert_eq!(back[1].value, "Yes");
        assert_eq!(back[2].value, "g");
        assert_eq!(back[3].value, "L");
        let doc = Document::load(&out).unwrap();
        let w = doc.get_dictionary(back[1].widgets[0].id).unwrap();
        assert_eq!(w.get(b"AS").unwrap().as_name().unwrap(), b"Yes");
        let small = doc.get_dictionary(back[3].widgets[0].id).unwrap();
        assert_eq!(small.get(b"AS").unwrap().as_name().unwrap(), b"Off");
        let name = doc.get_dictionary(back[0].widgets[0].id).unwrap();
        let ap = name
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap();
        let s = doc.get_object(ap).unwrap().as_stream().unwrap();
        assert!(String::from_utf8_lossy(&s.content).contains("(Ada Lovelace) Tj"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
