//! X11: the window, keys, full screen, files dropped on the window (XDND)
//! and a picture (image/png) or text from a PDF on the clipboard.
use crate::canvas::Canvas;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Atoms {
    pub wm_protocols: Atom,
    pub wm_delete: Atom,
    pub net_wm_name: Atom,
    pub net_wm_pid: Atom,
    pub net_wm_state: Atom,
    pub net_wm_state_fullscreen: Atom,
    pub utf8: Atom,
    pub niruwm_theme: Atom,
    pub clipboard: Atom,
    pub targets: Atom,
    pub png: Atom,
    pub uri_list: Atom,
    pub xdnd_aware: Atom,
    pub xdnd_position: Atom,
    pub xdnd_status: Atom,
    pub xdnd_drop: Atom,
    pub xdnd_finished: Atom,
    pub xdnd_selection: Atom,
    pub xdnd_copy: Atom,
    pub dnd_prop: Atom,
}

pub struct X {
    pub c: RustConnection,
    pub root: Window,
    pub depth: u8,
    gc: Gcontext,
    pub atoms: Atoms,
    pub win: Window,
    pub fullscreen: bool,
    min_keycode: u8,
    per: u8,
    keysyms: Vec<u32>,
}

fn atom(c: &RustConnection, name: &str) -> Res<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

impl X {
    pub fn open(title: &str) -> Res<Self> {
        let (c, screen) = x11rb::connect(None)?;
        let s = &c.setup().roots[screen];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let (sw, sh) = (s.width_in_pixels, s.height_in_pixels);
        let atoms = Atoms {
            wm_protocols: atom(&c, "WM_PROTOCOLS")?,
            wm_delete: atom(&c, "WM_DELETE_WINDOW")?,
            net_wm_name: atom(&c, "_NET_WM_NAME")?,
            net_wm_pid: atom(&c, "_NET_WM_PID")?,
            net_wm_state: atom(&c, "_NET_WM_STATE")?,
            net_wm_state_fullscreen: atom(&c, "_NET_WM_STATE_FULLSCREEN")?,
            utf8: atom(&c, "UTF8_STRING")?,
            niruwm_theme: atom(&c, "_NIRUWM_THEME")?,
            clipboard: atom(&c, "CLIPBOARD")?,
            targets: atom(&c, "TARGETS")?,
            png: atom(&c, "image/png")?,
            uri_list: atom(&c, "text/uri-list")?,
            xdnd_aware: atom(&c, "XdndAware")?,
            xdnd_position: atom(&c, "XdndPosition")?,
            xdnd_status: atom(&c, "XdndStatus")?,
            xdnd_drop: atom(&c, "XdndDrop")?,
            xdnd_finished: atom(&c, "XdndFinished")?,
            xdnd_selection: atom(&c, "XdndSelection")?,
            xdnd_copy: atom(&c, "XdndActionCopy")?,
            dnd_prop: atom(&c, "_NIRUEDITVIEW_DND")?,
        };
        let events = EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::STRUCTURE_NOTIFY;
        let win = c.generate_id()?;
        let (w, h) = ((sw as u32 * 2 / 3).max(640), (sh as u32 * 3 / 4).max(480));
        c.create_window(
            depth,
            win,
            root,
            0,
            0,
            w.min(sw as u32) as u16,
            h.min(sh as u32) as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(s.black_pixel)
                .bit_gravity(Gravity::NORTH_WEST)
                .event_mask(events),
        )?;
        c.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirueditview\0NIRUEDITVIEW\0",
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.net_wm_pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.wm_protocols,
            AtomEnum::ATOM,
            &[atoms.wm_delete],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.xdnd_aware,
            AtomEnum::ATOM,
            &[5],
        )?;
        let gc = c.generate_id()?;
        c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;
        let setup = c.setup();
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);
        let map = c
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;
        let x = X {
            c,
            root,
            depth,
            gc,
            atoms,
            win,
            fullscreen: false,
            min_keycode,
            per: map.keysyms_per_keycode,
            keysyms: map.keysyms,
        };
        x.set_title(title);
        x.c.map_window(win)?;
        x.c.flush()?;
        Ok(x)
    }

    pub fn set_title(&self, t: &str) {
        for (prop, kind) in [
            (self.atoms.net_wm_name, self.atoms.utf8),
            (AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()),
        ] {
            let _ = self
                .c
                .change_property8(PropMode::REPLACE, self.win, prop, kind, t.as_bytes());
        }
        self.flush();
    }

    pub fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd;
        self.c.stream().as_raw_fd()
    }

    pub fn poll(&self) -> Option<Event> {
        self.c.poll_for_event().ok().flatten()
    }

    pub fn flush(&self) {
        let _ = self.c.flush();
    }

    /// Ask the window manager for full screen (or back), EWMH style.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        let ev = ClientMessageEvent::new(
            32,
            self.win,
            self.atoms.net_wm_state,
            [
                u32::from(self.fullscreen),
                self.atoms.net_wm_state_fullscreen,
                0,
                1,
                0,
            ],
        );
        let _ = self.c.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        );
        self.flush();
    }

    /// Copy a canvas to the window, in slices that fit the request size.
    pub fn present(&self, cv: &Canvas) {
        let bytes = cv.as_bytes();
        let max = self.c.maximum_request_bytes().saturating_sub(64).max(4096);
        let row_bytes = cv.w as usize * 4;
        let rows = (max / row_bytes.max(1)).max(1);
        let mut y = 0usize;
        while y < cv.h as usize {
            let n = rows.min(cv.h as usize - y);
            let _ = self.c.put_image(
                ImageFormat::Z_PIXMAP,
                self.win,
                self.gc,
                cv.w as u16,
                n as u16,
                0,
                y as i16,
                0,
                self.depth,
                &bytes[y * row_bytes..(y + n) * row_bytes],
            );
            y += n;
        }
        self.flush();
    }

    pub fn root_theme(&self) -> Option<String> {
        let r = self
            .c
            .get_property(
                false,
                self.root,
                self.atoms.niruwm_theme,
                AtomEnum::ANY,
                0,
                64,
            )
            .ok()?
            .reply()
            .ok()?;
        let s = String::from_utf8(r.value).ok()?;
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn keysym(&self, keycode: u8, state: KeyButMask) -> u32 {
        let i = (keycode.saturating_sub(self.min_keycode)) as usize * self.per as usize;
        let shift = state.contains(KeyButMask::SHIFT);
        let get = |k: usize| self.keysyms.get(i + k).copied().unwrap_or(0);
        let low = get(0);
        let high = get(1);
        if shift && high != 0 {
            high
        } else if shift && (0x61..=0x7a).contains(&low) {
            low - 0x20
        } else {
            low
        }
    }

    // ------------------------------------------------------------ clipboard

    pub fn own_clipboard(&self) {
        let _ = self
            .c
            .set_selection_owner(self.win, self.atoms.clipboard, x11rb::CURRENT_TIME);
        self.flush();
    }

    /// Answer a paste: with the PNG bytes of a picture, or with text (from
    /// a PDF) when there is text. Large pictures go in one property when the
    /// server takes it (most do up to many megabytes).
    pub fn answer(&self, e: &SelectionRequestEvent, png: &[u8], text: &str) {
        let a = &self.atoms;
        let prop = if e.property == x11rb::NONE {
            e.target
        } else {
            e.property
        };
        let string: Atom = AtomEnum::STRING.into();
        let ok = if e.target == a.targets {
            let offer = if text.is_empty() {
                vec![a.targets, a.png]
            } else {
                vec![a.targets, a.utf8, string]
            };
            self.c
                .change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &offer)
                .is_ok()
        } else if (e.target == a.utf8 || e.target == string) && !text.is_empty() {
            let max = self.c.maximum_request_bytes().saturating_sub(64);
            text.len() <= max
                && self
                    .c
                    .change_property8(
                        PropMode::REPLACE,
                        e.requestor,
                        prop,
                        e.target,
                        text.as_bytes(),
                    )
                    .is_ok()
        } else if e.target == a.png && !png.is_empty() && text.is_empty() {
            let max = self.c.maximum_request_bytes().saturating_sub(64);
            png.len() <= max
                && self
                    .c
                    .change_property8(PropMode::REPLACE, e.requestor, prop, a.png, png)
                    .is_ok()
        } else {
            false
        };
        let ev = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if ok { prop } else { x11rb::NONE },
        };
        let _ = self
            .c
            .send_event(false, e.requestor, EventMask::NO_EVENT, ev);
        self.flush();
    }

    // ------------------------------------------------------------ dropping

    pub fn send_client(&self, to: Window, kind: Atom, data: [u32; 5]) {
        let ev = ClientMessageEvent::new(32, to, kind, data);
        let _ = self.c.send_event(false, to, EventMask::NO_EVENT, ev);
        self.flush();
    }

    pub fn request_drop(&self, time: u32) {
        let _ = self.c.convert_selection(
            self.win,
            self.atoms.xdnd_selection,
            self.atoms.uri_list,
            self.atoms.dnd_prop,
            time,
        );
        self.flush();
    }

    pub fn take_drop(&self) -> Vec<u8> {
        self.c
            .get_property(
                true,
                self.win,
                self.atoms.dnd_prop,
                AtomEnum::ANY,
                0,
                1 << 24,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value)
            .unwrap_or_default()
    }
}

/// Local paths from a text/uri-list.
pub fn parse_uris(data: &[u8]) -> Vec<std::path::PathBuf> {
    String::from_utf8_lossy(data)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let rest = l.strip_prefix("file://")?;
            let path = rest.find('/').map(|i| &rest[i..])?;
            Some(std::path::PathBuf::from(percent_decode(path)))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Keysyms used by the app.
pub mod keys {
    pub const ESCAPE: u32 = 0xff1b;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const BACKSPACE: u32 = 0xff08;
    pub const DELETE: u32 = 0xffff;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PRIOR: u32 = 0xff55;
    pub const NEXT: u32 = 0xff56;
    pub const HOME: u32 = 0xff50;
    pub const END: u32 = 0xff57;
    pub const F1: u32 = 0xffbe;
    pub const F11: u32 = 0xffc8;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SUBTRACT: u32 = 0xffad;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_uris_become_paths() {
        let p = parse_uris(b"file:///home/a/My%20Pictures/x.png\r\nfile://host/tmp/y.pdf\n# c\n");
        assert_eq!(
            p,
            vec![
                std::path::PathBuf::from("/home/a/My Pictures/x.png"),
                std::path::PathBuf::from("/tmp/y.pdf")
            ]
        );
    }
}
