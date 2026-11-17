//! Desktop app. The UI lives in `app/src`, these commands do all disk and
//! rendering work through the `book-*` crates.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use book_model::Book;
use book_project::{ChapterFile, Meta, Node};
use serde::Serialize;

#[derive(Serialize)]
struct Project {
    dir: PathBuf,
    meta: Meta,
    chapters: Vec<ChapterFile>,
}

#[derive(Serialize)]
struct Options {
    styles: Vec<&'static str>,
    trims: Vec<(&'static str, f64, f64)>,
    fonts: Vec<String>,
}

#[tauri::command]
fn options() -> Options {
    Options {
        styles: book_typeset::STYLES.iter().map(|(n, _)| *n).collect(),
        trims: book_typeset::TRIMS.to_vec(),
        fonts: book_typeset::font_families(),
    }
}

#[tauri::command]
fn open_project(dir: PathBuf) -> Result<Project, String> {
    let (meta, chapters) = book_project::load(&dir)?;
    Ok(Project {
        dir,
        meta,
        chapters,
    })
}

/// Creates a project in `dir`, empty or from a Markdown manuscript.
#[tauri::command]
fn new_project(dir: PathBuf, title: String, markdown: Option<PathBuf>) -> Result<Project, String> {
    let chapters = match &markdown {
        Some(path) => book_import::markdown(
            &std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
        ),
        None => Vec::new(),
    };
    let book = Book {
        title,
        author: String::new(),
        language: String::new(),
        copyright: String::new(),
        chapters,
    };
    book_project::create(&dir, &book)?;
    open_project(dir)
}

#[tauri::command]
fn save_meta(dir: PathBuf, meta: Meta) -> Result<(), String> {
    book_project::save_meta(&dir, &meta)
}

/// While the author writes, a snapshot is taken at most this often.
const SNAPSHOT_EVERY_SECS: u64 = 10 * 60;

#[tauri::command]
/// Returns a warning when the chapter is saved but the snapshot failed. Not an
/// error: the text is on disk, and an error would make the app refuse to close.
fn save_chapter(
    dir: PathBuf,
    file: String,
    chapter: ChapterFile,
) -> Result<Option<String>, String> {
    book_project::save_chapter(&dir, &file, &chapter)?;
    Ok(book_project::snapshot_if_due(&dir, SNAPSHOT_EVERY_SECS).err())
}

#[tauri::command]
fn snapshots(dir: PathBuf) -> Result<Vec<book_project::Snapshot>, String> {
    book_project::snapshots(&dir)
}

#[tauri::command]
fn restore_snapshot(dir: PathBuf, id: u64) -> Result<Project, String> {
    book_project::restore(&dir, id)?;
    open_project(dir)
}

#[tauri::command]
fn drafts(dir: PathBuf) -> Result<Vec<String>, String> {
    book_project::drafts(&dir)
}

#[tauri::command]
fn new_draft(dir: PathBuf, name: String) -> Result<Project, String> {
    book_project::new_draft(&dir, &name)?;
    open_project(dir)
}

#[tauri::command]
fn switch_draft(dir: PathBuf, name: String) -> Result<Project, String> {
    book_project::switch_draft(&dir, &name)?;
    open_project(dir)
}

/// Appends an empty chapter and returns the project as saved.
#[tauri::command]
fn add_chapter(dir: PathBuf) -> Result<Project, String> {
    let (mut meta, _) = book_project::load(&dir)?;
    let file = book_project::new_chapter_file(&meta);
    let empty = ChapterFile {
        title: String::new(),
        doc: Node {
            kind: "doc".into(),
            ..Node::default()
        },
    };
    book_project::save_chapter(&dir, &file, &empty)?;
    meta.chapters.push(file);
    book_project::save_meta(&dir, &meta)?;
    open_project(dir)
}

fn typst_source(meta: &Meta, chapters: &[ChapterFile]) -> Result<String, String> {
    book_typeset::to_typst(
        &book_project::book(meta, chapters),
        &book_project::layout(meta),
    )
}

/// One SVG per page. Takes the editor state, so the preview never waits for a save.
#[tauri::command]
async fn preview(meta: Meta, chapters: Vec<ChapterFile>) -> Result<Vec<String>, String> {
    blocking(move || book_typeset::compile_svg(&typst_source(&meta, &chapters)?, None)).await
}

#[tauri::command]
async fn export(
    dir: PathBuf,
    meta: Meta,
    chapters: Vec<ChapterFile>,
    pdf: Option<PathBuf>,
    epub: Option<PathBuf>,
    cover: Option<PathBuf>,
) -> Result<(), String> {
    blocking(move || {
        if let Some(path) = cover {
            let pages = book_project::page_count(&meta, &chapters)?;
            let (source, image) = book_project::cover_source(&dir, &meta, &chapters, pages)?;
            write(&path, book_typeset::compile_pdf(&source, image.as_deref())?)?;
        }
        if let Some(path) = pdf {
            write(
                &path,
                book_typeset::compile_pdf(&typst_source(&meta, &chapters)?, None)?,
            )?;
        }
        if let Some(path) = epub {
            write(
                &path,
                book_epub::to_epub(&book_project::book(&meta, &chapters), &meta.style)?,
            )?;
        }
        Ok(())
    })
    .await
}

/// The full cover as SVG, bleed included.
#[tauri::command]
async fn cover_preview(
    dir: PathBuf,
    meta: Meta,
    chapters: Vec<ChapterFile>,
) -> Result<String, String> {
    blocking(move || {
        let pages = book_project::page_count(&meta, &chapters)?;
        let (source, image) = book_project::cover_source(&dir, &meta, &chapters, pages)?;
        let svg = book_typeset::compile_svg(&source, image.as_deref())?;
        svg.into_iter().next().ok_or_else(|| "empty cover".into())
    })
    .await
}

/// Book Health: what a printer or store would refuse, and what looks wrong.
#[tauri::command]
async fn check(
    dir: PathBuf,
    meta: Meta,
    chapters: Vec<ChapterFile>,
) -> Result<Vec<book_preflight::Issue>, String> {
    blocking(move || book_project::check(&dir, &meta, &chapters)).await
}

/// Copies an image into the project and returns its name for `meta.cover.image`.
#[tauri::command]
fn import_cover_image(dir: PathBuf, path: PathBuf) -> Result<String, String> {
    book_project::import_cover_image(&dir, &path)
}

fn write(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Typesetting takes up to seconds: keep it off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// `lettrine <project-dir>` opens that project on start.
#[tauri::command]
fn startup_project() -> Option<PathBuf> {
    std::env::args_os().nth(1).map(PathBuf::from)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            options,
            open_project,
            new_project,
            save_meta,
            save_chapter,
            add_chapter,
            snapshots,
            restore_snapshot,
            drafts,
            new_draft,
            switch_draft,
            preview,
            export,
            startup_project,
            cover_preview,
            check,
            import_cover_image,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the app");
}
