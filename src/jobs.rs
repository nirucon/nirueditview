//! Work off the window's thread: pictures decoded (the one asked for first,
//! then its neighbours, so turning a page is instant), PDF pages drawn,
//! thumbnails made, the words of a PDF read and the wallpaper set.
//! Results come back on a channel, and a pipe wakes the window's poll.
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Job {
    /// Decode a picture. `preload` jobs give way to everything else.
    Picture {
        path: PathBuf,
        preload: bool,
    },
    /// Draw page `page` (0-based) at `dpi`.
    Page {
        path: PathBuf,
        page: usize,
        dpi: u32,
    },
    Thumb {
        path: PathBuf,
    },
    /// Read the words of a PDF (to find and copy text).
    Words {
        path: PathBuf,
    },
    /// Set a picture as the desktop wallpaper with NIRUWALLPAPER.
    Wallpaper {
        path: PathBuf,
    },
}

impl Job {
    fn urgent(&self) -> u8 {
        match self {
            Job::Picture { preload: false, .. } | Job::Wallpaper { .. } => 0,
            Job::Page { .. } | Job::Words { .. } => 1,
            Job::Thumb { .. } => 2,
            Job::Picture { preload: true, .. } => 3,
        }
    }
}

pub enum Done {
    Picture(PathBuf, Result<crate::picture::Picture, String>),
    Page(PathBuf, usize, u32, Result<crate::pdf::Raster, String>),
    Thumb(PathBuf, Option<image::RgbaImage>),
    Words(PathBuf, Result<crate::pdftext::Words, String>),
    Wallpaper(PathBuf, Result<(), String>),
}

/// The NIRUWALLPAPER command (`NIRUEDITVIEW_NIRUWALLPAPER` names another,
/// for tests).
pub fn wallpaper_command() -> PathBuf {
    std::env::var_os("NIRUEDITVIEW_NIRUWALLPAPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| "niruwallpaper".into())
}

/// `niruwallpaper set PATH`: NIRUWALLPAPER hands the picture to niructl,
/// which keeps it in the shared settings and redraws the desktop.
pub fn set_wallpaper(path: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new(wallpaper_command())
        .arg("set")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "niruwallpaper is not installed (it comes with NIRUX)".to_string()
            }
            _ => format!("niruwallpaper: {e}"),
        })?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .last()
            .unwrap_or("niruwallpaper failed")
            .trim_start_matches("niruwallpaper: ")
            .to_string())
    }
}

struct Queue {
    jobs: Vec<Job>,
    busy: Vec<Job>,
}

pub struct Jobs {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    pub rx: Receiver<Done>,
    pub wake_fd: i32,
    wake_w: i32,
}

fn poke(fd: i32) {
    let b = [1u8];
    unsafe { libc::write(fd, b.as_ptr() as *const libc::c_void, 1) };
}

impl Jobs {
    pub fn start(threads: usize) -> Self {
        let mut fds = [0i32; 2];
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        let queue = Arc::new((
            Mutex::new(Queue {
                jobs: Vec::new(),
                busy: Vec::new(),
            }),
            Condvar::new(),
        ));
        let (tx, rx): (Sender<Done>, Receiver<Done>) = std::sync::mpsc::channel();
        for i in 0..threads.max(1) {
            let (q, tx, w) = (queue.clone(), tx.clone(), fds[1]);
            let _ = std::thread::Builder::new()
                .name(format!("nirueditview-{i}"))
                .spawn(move || loop {
                    let job = {
                        let (lock, cv) = &*q;
                        let Ok(mut g) = lock.lock() else { return };
                        loop {
                            if !g.jobs.is_empty() {
                                // The most urgent; among equals the newest.
                                let best = (0..g.jobs.len())
                                    .rev()
                                    .min_by_key(|i| g.jobs[*i].urgent())
                                    .unwrap_or(0);
                                let j = g.jobs.remove(best);
                                g.busy.push(j.clone());
                                break j;
                            }
                            g = match cv.wait(g) {
                                Ok(g) => g,
                                Err(_) => return,
                            };
                        }
                    };
                    let done = match &job {
                        Job::Picture { path, .. } => {
                            Done::Picture(path.clone(), crate::picture::load(path))
                        }
                        Job::Page { path, page, dpi } => Done::Page(
                            path.clone(),
                            *page,
                            *dpi,
                            crate::pdf::render(path, page + 1, *dpi),
                        ),
                        Job::Thumb { path } => {
                            Done::Thumb(path.clone(), crate::files::thumbnail(path))
                        }
                        Job::Words { path } => {
                            Done::Words(path.clone(), crate::pdftext::read(path))
                        }
                        Job::Wallpaper { path } => {
                            Done::Wallpaper(path.clone(), set_wallpaper(path))
                        }
                    };
                    if let Ok(mut g) = q.0.lock() {
                        if let Some(i) = g.busy.iter().position(|b| *b == job) {
                            g.busy.remove(i);
                        }
                    }
                    if tx.send(done).is_err() {
                        return;
                    }
                    poke(w);
                });
        }
        Self {
            queue,
            rx,
            wake_fd: fds[0],
            wake_w: fds[1],
        }
    }

    /// Queue a job unless the same one is queued or running.
    pub fn ask(&self, job: Job) {
        let (lock, cv) = &*self.queue;
        if let Ok(mut g) = lock.lock() {
            if g.jobs.contains(&job) || g.busy.contains(&job) {
                return;
            }
            g.jobs.push(job);
        }
        cv.notify_one();
    }

    /// Forget queued work of one kind (thumbnails of a closed gallery,
    /// preloads of a folder left behind).
    pub fn drop_where(&self, f: impl Fn(&Job) -> bool) {
        if let Ok(mut g) = self.queue.0.lock() {
            g.jobs.retain(|j| !f(j));
        }
    }

    pub fn drain_wake(&self) {
        let mut buf = [0u8; 64];
        while unsafe {
            libc::read(
                self.wake_fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        } > 0
        {}
    }

    /// A wake-up from the window's own thread (a timer elsewhere).
    #[allow(dead_code)]
    pub fn wake(&self) {
        poke(self.wake_w);
    }
}
