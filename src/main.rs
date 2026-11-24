//! NIRUEDITVIEW: pictures and PDFs for NIRUX. A fast viewer with a gallery
//! and a slideshow, simple picture edits, and signatures, text and dates
//! put on PDFs. Saved changes go to a copy unless you ask otherwise.
mod app;
mod canvas;
mod files;
mod jobs;
mod pdf;
mod picture;
mod text;
mod theme;
mod ui;
mod x;

use std::process::ExitCode;

const HELP: &str = "\
nirueditview [FILE|FOLDER]...   show pictures and PDFs (a folder opens its gallery)
nirueditview --version

Keys: ? in the window. Pictures: arrows turn, G gallery, S slideshow, R rotate,
C crop, P pen, X pixelate, Ctrl+S saves a copy. PDF: S signature, T text,
D date, Ctrl+S saves a signed copy. The original is kept unless you choose
Ctrl+Shift+S (pictures).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version" | "version") => {
            println!("nirueditview {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let paths: Vec<std::path::PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| {
            // A file:// address from a file manager.
            if a.starts_with("file://") {
                x::parse_uris(a.as_bytes())
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| a.into())
            } else {
                a.into()
            }
        })
        .collect();
    match app::App::new(paths) {
        Ok(mut a) => {
            a.refit();
            a.run();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nirueditview: {e}");
            ExitCode::FAILURE
        }
    }
}
