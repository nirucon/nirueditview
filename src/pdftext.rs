//! The words of a PDF, read with Poppler (`pdftotext -bbox-layout`) only
//! when they are asked for: to find text (Ctrl+F) and to select and copy
//! it. Boxes are in points on the page as shown (rotation applied, from
//! the top-left corner of the crop box), the same space the stamps use.
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    /// Shown points: left, top, right, bottom.
    pub b: [f64; 4],
    /// Which line of the page it is on (counted over the page).
    pub line: usize,
}

/// The words of every page, in reading order.
pub type Words = Vec<Vec<Word>>;

fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag[at..].find('"')? + at;
    tag[at..end].parse().ok()
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read what `pdftotext -bbox-layout` writes.
pub fn parse_bbox(xml: &str) -> Words {
    let mut pages: Words = Vec::new();
    let mut line = 0usize;
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        if tag.starts_with("<page ") || tag == "<page>" {
            pages.push(Vec::new());
            line = 0;
        } else if tag.starts_with("<line ") {
            line += 1;
        } else if tag.starts_with("<word ") {
            let body_end = rest[end + 1..].find("</word>").map(|e| e + end + 1);
            let text = body_end
                .map(|e| unescape(&rest[end + 1..e]))
                .unwrap_or_default();
            if let (Some(x0), Some(y0), Some(x1), Some(y1), Some(page)) = (
                attr(tag, "xMin"),
                attr(tag, "yMin"),
                attr(tag, "xMax"),
                attr(tag, "yMax"),
                pages.last_mut(),
            ) {
                if !text.is_empty() {
                    page.push(Word {
                        text,
                        b: [x0, y0, x1, y1],
                        line: line.saturating_sub(1),
                    });
                }
            }
            if let Some(e) = body_end {
                rest = &rest[e..];
                continue;
            }
        }
        rest = &rest[end + 1..];
    }
    pages
}

pub fn read(path: &Path) -> Result<Words, String> {
    let out = Command::new("pdftotext")
        .args(["-bbox-layout", "-cropbox", "-enc", "UTF-8"])
        .arg(path)
        .arg("-")
        .output()
        .map_err(|e| format!("pdftotext: {e} (install poppler)"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("pdftotext failed").to_string());
    }
    Ok(parse_bbox(&String::from_utf8_lossy(&out.stdout)))
}

/// One place the text was found: its page and the boxes to light up (part
/// of a word when the match starts or ends inside one).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub boxes: Vec<[f64; 4]>,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Every place `needle` is, page by page, without regard to case; words
/// are joined with one space, so a phrase over two words (or two lines) is
/// found too.
pub fn find(words: &Words, needle: &str) -> Vec<Hit> {
    let needle: Vec<char> = needle.trim().chars().map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (page, ws) in words.iter().enumerate() {
        // The page as one string, and for each character its word and its
        // place in the word (None: the space between words).
        let mut hay: Vec<char> = Vec::new();
        let mut at: Vec<Option<(usize, usize)>> = Vec::new();
        for (wi, w) in ws.iter().enumerate() {
            if wi > 0 {
                hay.push(' ');
                at.push(None);
            }
            for (ci, c) in w.text.chars().enumerate() {
                hay.push(fold(c));
                at.push(Some((wi, ci)));
            }
        }
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                let mut boxes = Vec::new();
                let mut span: Option<(usize, usize, usize)> = None;
                let mut flush = |s: Option<(usize, usize, usize)>| {
                    if let Some((wi, a, b)) = s {
                        boxes.push(part(&ws[wi], a, b));
                    }
                };
                for &(wi, ci) in at[i..i + needle.len()].iter().flatten() {
                    match span {
                        Some((w, a, _)) if w == wi => span = Some((w, a, ci)),
                        _ => {
                            flush(span);
                            span = Some((wi, ci, ci));
                        }
                    }
                }
                flush(span);
                hits.push(Hit { page, boxes });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    hits
}

/// The box of characters `a..=b` of a word, by their share of its width
/// (vertical words, on a page turned by the file, share the height).
fn part(w: &Word, a: usize, b: usize) -> [f64; 4] {
    let n = w.text.chars().count().max(1) as f64;
    let [x0, y0, x1, y1] = w.b;
    let (fa, fb) = (a as f64 / n, (b + 1) as f64 / n);
    if (y1 - y0) > (x1 - x0) * 1.5 && n > 1.0 {
        [x0, y0 + (y1 - y0) * fa, x1, y0 + (y1 - y0) * fb]
    } else {
        [x0 + (x1 - x0) * fa, y0, x0 + (x1 - x0) * fb, y1]
    }
}

/// The word nearest a point on a page (shown points).
pub fn nearest(ws: &[Word], u: f64, v: f64) -> Option<usize> {
    let dist = |w: &Word| {
        let dx = (w.b[0] - u).max(u - w.b[2]).max(0.0);
        let dy = (w.b[1] - v).max(v - w.b[3]).max(0.0);
        dx * dx + dy * dy * 4.0
    };
    (0..ws.len()).min_by(|a, b| dist(&ws[*a]).total_cmp(&dist(&ws[*b])))
}

/// The words from the one at `a` to the one at `b` in reading order.
pub fn range(ws: &[Word], a: (f64, f64), b: (f64, f64)) -> Option<(usize, usize)> {
    let i = nearest(ws, a.0, a.1)?;
    let j = nearest(ws, b.0, b.1)?;
    Some((i.min(j), i.max(j)))
}

/// The text of words `a..=b`: a space between words, a new line between
/// lines.
pub fn text_of(ws: &[Word], a: usize, b: usize) -> String {
    let mut out = String::new();
    for (k, w) in ws.iter().enumerate().take(b + 1).skip(a) {
        if k > a {
            out.push(if w.line != ws[k - 1].line { '\n' } else { ' ' });
        }
        out.push_str(&w.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<doc>
  <page width="595.000000" height="842.000000">
    <flow><block xMin="72" yMin="54" xMax="300" yMax="100">
        <line xMin="72" yMin="54" xMax="300" yMax="76">
          <word xMin="72.000000" yMin="54.000000" xMax="118.000000" yMax="76.000000">Test</word>
          <word xMin="125.000000" yMin="54.000000" xMax="173.000000" yMax="76.000000">form</word>
        </line>
        <line xMin="72" yMin="80" xMax="300" yMax="100">
          <word xMin="72.000000" yMin="80.000000" xMax="140.000000" yMax="100.000000">R&amp;D&#xe5;</word>
        </line>
    </block></flow>
  </page>
  <page width="595.000000" height="842.000000">
    <flow><block><line>
      <word xMin="765.0" yMin="72.0" xMax="787.0" yMax="128.0">Page</word>
      <word xMin="765.0" yMin="134.0" xMax="787.0" yMax="172.0">two</word>
    </line></block></flow>
  </page>
</doc>"#;

    #[test]
    fn pdftotext_words_are_read_with_their_boxes_and_lines() {
        let w = parse_bbox(XML);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].len(), 3);
        assert_eq!(w[0][2].text, "R&Då");
        assert_eq!(w[0][0].line, 0);
        assert_eq!(w[0][2].line, 1);
        assert_eq!(w[1][0].b, [765.0, 72.0, 787.0, 128.0]);
    }

    #[test]
    fn find_ignores_case_and_spans_words_and_pages() {
        let w = parse_bbox(XML);
        let h = find(&w, "test FORM");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].page, 0);
        assert_eq!(h[0].boxes.len(), 2);
        // Half of "Test": the first two of four letters.
        let h = find(&w, "te");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].boxes[0], [72.0, 54.0, 95.0, 76.0]);
        let h = find(&w, "page");
        assert_eq!(h[0].page, 1);
        // A word on a turned page: the share is of its height.
        let h = find(&w, "pa");
        assert_eq!(h[0].boxes[0], [765.0, 72.0, 787.0, 100.0]);
        assert!(find(&w, "").is_empty());
        assert!(find(&w, "nowhere").is_empty());
    }

    #[test]
    fn a_selection_runs_in_reading_order_with_lines() {
        let w = parse_bbox(XML);
        let (a, b) = range(&w[0], (130.0, 60.0), (70.0, 90.0)).unwrap();
        assert_eq!((a, b), (1, 2));
        assert_eq!(text_of(&w[0], 0, 2), "Test form\nR&Då");
    }
}
