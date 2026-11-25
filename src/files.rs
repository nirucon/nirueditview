//! The folder being looked through: its pictures and PDFs in natural order
//! (or by date or size), the trash for a picture you no longer want, and
//! thumbnails shared with NIRUFM and every other file manager
//! (`~/.cache/thumbnails/large/MD5(uri).png`).
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Date,
    Size,
}

impl Sort {
    pub fn next(self) -> Self {
        match self {
            Sort::Name => Sort::Date,
            Sort::Date => Sort::Size,
            Sort::Size => Sort::Name,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Date => "date",
            Sort::Size => "size",
        }
    }
}

pub fn shown(p: &Path) -> bool {
    crate::picture::is_picture(p) || crate::picture::is_pdf(p)
}

/// "img2" before "img10", case ignored.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n2, m2) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n2.len().cmp(&m2.len()).then_with(|| n2.cmp(m2));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// The pictures and PDFs of `dir`, hidden files left out.
pub fn list(dir: &Path, sort: Sort) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(PathBuf, i64, u64)> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let p = e.path();
            if !shown(&p) {
                return None;
            }
            let m = std::fs::metadata(&p).ok()?;
            if !m.is_file() {
                return None;
            }
            use std::os::unix::fs::MetadataExt;
            Some((p, m.mtime(), m.len()))
        })
        .collect();
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match sort {
        Sort::Name => v.sort_by(|a, b| natural_cmp(&name(&a.0), &name(&b.0))),
        Sort::Date => v.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| natural_cmp(&name(&a.0), &name(&b.0)))
        }),
        Sort::Size => v.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| natural_cmp(&name(&a.0), &name(&b.0)))
        }),
    }
    v.into_iter().map(|e| e.0).collect()
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/x` typed in a question is the home folder's `x`.
pub fn expand_home(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) => p.to_path_buf(),
    }
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
}

fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
}

pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Move `p` to the trash (the freedesktop trash in the home folder, with
/// the info file that lets a file manager put it back). A file on another
/// disk goes through `gio trash` when it is there; otherwise it stays.
pub fn trash(p: &Path) -> Result<(), String> {
    let p = std::fs::canonicalize(p).map_err(|e| e.to_string())?;
    let base = data_home().join("Trash");
    let (files, info) = (base.join("files"), base.join("info"));
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("no file name")?;
    let mut n = 1;
    let (target, info_file) = loop {
        let candidate = if n == 1 {
            name.clone()
        } else {
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (name.clone(), String::new()),
            };
            format!("{stem}.{n}{ext}")
        };
        let t = files.join(&candidate);
        let i = info.join(format!("{candidate}.trashinfo"));
        if !t.exists() && !i.exists() {
            break (t, i);
        }
        n += 1;
    };
    let now = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    };
    let uri = file_uri(&p);
    let path_field = uri.trim_start_matches("file://");
    std::fs::write(
        &info_file,
        format!("[Trash Info]\nPath={path_field}\nDeletionDate={now}\n"),
    )
    .map_err(|e| e.to_string())?;
    match std::fs::rename(&p, &target) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            let _ = std::fs::remove_file(&info_file);
            let ok = std::process::Command::new("gio")
                .arg("trash")
                .arg(&p)
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                Ok(())
            } else {
                Err("the file is on another disk and gio is not there to trash it".into())
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&info_file);
            Err(e.to_string())
        }
    }
}

// ------------------------------------------------------------ thumbnails

fn thumb_name(p: &Path) -> String {
    format!("{:x}.png", md5::compute(file_uri(p).as_bytes()))
}

fn mtime(p: &Path) -> Option<i64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.mtime())
}

/// A tEXt chunk value from PNG bytes.
fn png_text(data: &[u8], key: &str) -> Option<String> {
    let mut i = 8;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
        let kind = &data[i + 4..i + 8];
        let body = data.get(i + 8..i + 8 + len)?;
        if kind == b"tEXt" {
            if let Some(z) = body.iter().position(|b| *b == 0) {
                if &body[..z] == key.as_bytes() {
                    return Some(String::from_utf8_lossy(&body[z + 1..]).into_owned());
                }
            }
        }
        if kind == b"IDAT" {
            break;
        }
        i += 12 + len;
    }
    None
}

/// PNG bytes with Thumb::URI and Thumb::MTime after the header, as the
/// thumbnail specification asks.
fn with_stamp(png: &[u8], uri: &str, mtime: i64) -> Vec<u8> {
    if png.len() < 33 {
        return png.to_vec();
    }
    let mut out = png[..33].to_vec();
    for (k, v) in [
        ("Thumb::URI", uri.to_string()),
        ("Thumb::MTime", mtime.to_string()),
    ] {
        let mut chunk = b"tEXt".to_vec();
        chunk.extend_from_slice(k.as_bytes());
        chunk.push(0);
        chunk.extend_from_slice(v.as_bytes());
        out.extend_from_slice(&((chunk.len() - 4) as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(&chunk);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }
    out.extend_from_slice(&png[33..]);
    out
}

/// A thumbnail of at most 256 pixels: from the shared cache when it is
/// fresh, else made and written there.
pub fn thumbnail(p: &Path) -> Option<image::RgbaImage> {
    let mt = mtime(p)?;
    let cached = cache_home().join("thumbnails/large").join(thumb_name(p));
    if let Ok(data) = std::fs::read(&cached) {
        if png_text(&data, "Thumb::MTime").and_then(|v| v.parse::<i64>().ok()) == Some(mt) {
            if let Ok(img) = image::load_from_memory(&data) {
                return Some(img.into_rgba8());
            }
        }
    }
    let img = if crate::picture::is_pdf(p) {
        let r = crate::pdf::render(p, 1, 36).ok()?;
        let rgb = image::RgbImage::from_raw(r.w, r.h, r.rgb)?;
        image::DynamicImage::ImageRgb8(rgb)
            .thumbnail(256, 256)
            .into_rgba8()
    } else {
        crate::picture::thumbnail(p, 256).ok()?
    };
    let png = crate::picture::png_bytes(&img);
    if let Some(dir) = cached.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            let tmp = cached.with_extension(format!("nirueditview-{}.png", std::process::id()));
            if std::fs::write(&tmp, with_stamp(&png, &file_uri(p), mt)).is_ok() {
                let _ = std::fs::rename(&tmp, &cached);
            }
        }
    }
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_counts_numbers() {
        let mut v = vec!["img10.jpg", "IMG2.jpg", "img1.jpg", "a.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a.png", "img1.jpg", "IMG2.jpg", "img10.jpg"]);
    }

    #[test]
    fn thumbnails_are_shared_and_stamped() {
        let dir = std::env::temp_dir().join(format!("nev-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
        let p = dir.join("photo one.png");
        crate::picture::save(
            &image::RgbaImage::from_pixel(600, 300, image::Rgba([1, 2, 3, 255])),
            &p,
        )
        .unwrap();
        let t = thumbnail(&p).unwrap();
        assert_eq!(t.dimensions(), (256, 128));
        let cached = dir.join("cache/thumbnails/large").join(thumb_name(&p));
        let data = std::fs::read(&cached).unwrap();
        assert_eq!(png_text(&data, "Thumb::URI").unwrap(), file_uri(&p));
        assert!(file_uri(&p).ends_with("photo%20one.png"));
        assert!(image::load_from_memory(&data).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trash_keeps_an_info_file() {
        let dir = std::env::temp_dir().join(format!("nev-trash-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("pics")).unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        let p = dir.join("pics/x.jpg");
        std::fs::write(&p, b"x").unwrap();
        trash(&p).unwrap();
        assert!(!p.exists());
        assert!(dir.join("data/Trash/files/x.jpg").exists());
        let info = std::fs::read_to_string(dir.join("data/Trash/info/x.jpg.trashinfo")).unwrap();
        assert!(info.contains("Path=") && info.contains("pics/x.jpg"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
