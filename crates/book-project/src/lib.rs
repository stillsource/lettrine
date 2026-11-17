//! A book project on disk, and the editor document format.
//!
//! Layout of a project folder:
//! - `book.json`: metadata, layout settings and the chapter order.
//! - `chapters/<file>.json`: one chapter, `{ "title": ..., "doc": <ProseMirror JSON> }`.
//! - `drafts/<name>/`: the other drafts, each with its own `book.json` and `chapters/`.
//!   The active draft is the one at the root.
//! - `.snapshots/<unix seconds>/`: automatic copies of the active draft, same layout.
//! - `assets/`: files shared by all drafts, like the cover image. Not in snapshots.
//!
//! Chapters are stored as the editor's ProseMirror JSON, so the editor loads them
//! as is. The book AST is built from them on demand.

use std::fs;
use std::path::{Path, PathBuf};

use book_model::{Block, Book, Chapter, Inline};
use serde::{Deserialize, Serialize};

/// `book.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub copyright: String,
    #[serde(default = "default_style")]
    pub style: String,
    /// Trim size in inches (width, height).
    #[serde(default = "default_trim")]
    pub trim: (f64, f64),
    #[serde(default)]
    pub bleed: bool,
    /// Chapter file names under `chapters/`, in reading order.
    #[serde(default)]
    pub chapters: Vec<String>,
    /// Name of this draft.
    #[serde(default = "default_draft")]
    pub draft: String,
    /// Overrides of the style, for print. `None` keeps the style's value.
    #[serde(default)]
    pub font: Option<String>,
    /// Body text size in points.
    #[serde(default)]
    pub size: Option<f64>,
    /// Space between lines, in em.
    #[serde(default)]
    pub leading: Option<f64>,
    #[serde(default)]
    pub cover: CoverSettings,
}

/// Cover settings in `book.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoverSettings {
    /// "white", "cream" or "color": sets the spine width by KDP's formula.
    pub paper: String,
    /// Spine width in inches, when the printer gives it (IngramSpark's template does).
    /// Replaces the computed width.
    pub spine: Option<f64>,
    pub blurb: String,
    pub background: String,
    pub ink: String,
    /// Front image file name under `assets/`.
    pub image: Option<String>,
}

impl Default for CoverSettings {
    fn default() -> Self {
        Self {
            paper: "white".into(),
            spine: None,
            blurb: String::new(),
            background: "#2b2522".into(),
            ink: "#f6f3ee".into(),
            image: None,
        }
    }
}

fn default_draft() -> String {
    "Brouillon 1".into()
}

fn default_style() -> String {
    "classique".into()
}

fn default_trim() -> (f64, f64) {
    (5.5, 8.5)
}

/// One chapter file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterFile {
    pub title: String,
    pub doc: Node,
}

/// A ProseMirror node, as TipTap serialises it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    #[serde(rename = "type")]
    pub kind: String,
}

pub type Result<T> = std::result::Result<T, String>;

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

/// Reads `book.json` and every chapter of a project.
pub fn load(dir: &Path) -> Result<(Meta, Vec<ChapterFile>)> {
    let meta_path = dir.join("book.json");
    let meta: Meta = read_json(&meta_path)?;
    let chapters = meta
        .chapters
        .iter()
        .map(|file| read_json(&chapter_path(dir, file)?))
        .collect::<Result<_>>()?;
    Ok((meta, chapters))
}

/// The book AST of a project, ready to render.
pub fn book(meta: &Meta, chapters: &[ChapterFile]) -> Book {
    Book {
        title: meta.title.clone(),
        author: meta.author.clone(),
        language: meta.language.clone(),
        copyright: meta.copyright.clone(),
        chapters: chapters
            .iter()
            .map(|c| Chapter {
                title: c.title.clone(),
                blocks: blocks(&c.doc),
            })
            .collect(),
    }
}

/// Creates a new project folder from a book. The folder must not exist or be empty.
pub fn create(dir: &Path, book: &Book) -> Result<Meta> {
    if dir.exists() && fs::read_dir(dir).map_err(io_err(dir))?.next().is_some() {
        return Err(format!("{} is not empty", dir.display()));
    }
    fs::create_dir_all(dir.join("chapters")).map_err(io_err(dir))?;
    let chapters: Vec<Chapter> = if book.chapters.is_empty() {
        vec![Chapter {
            title: String::new(),
            blocks: Vec::new(),
        }]
    } else {
        book.chapters.clone()
    };
    let mut meta = Meta {
        title: book.title.clone(),
        author: book.author.clone(),
        language: book.language.clone(),
        copyright: book.copyright.clone(),
        style: default_style(),
        trim: default_trim(),
        bleed: false,
        chapters: Vec::new(),
        draft: default_draft(),
        font: None,
        size: None,
        leading: None,
        cover: CoverSettings::default(),
    };
    for chapter in &chapters {
        let file = new_chapter_file(&meta);
        save_chapter(
            dir,
            &file,
            &ChapterFile {
                title: chapter.title.clone(),
                doc: doc(&chapter.blocks),
            },
        )?;
        meta.chapters.push(file);
    }
    save_meta(dir, &meta)?;
    Ok(meta)
}

/// A file name not used yet by any chapter. Names never get reused, so a
/// file keeps its identity when chapters are reordered.
pub fn new_chapter_file(meta: &Meta) -> String {
    let next = meta
        .chapters
        .iter()
        .filter_map(|f| f.strip_suffix(".json")?.parse::<u32>().ok())
        .max()
        .map_or(1, |n| n + 1);
    format!("{next:03}.json")
}

pub fn save_meta(dir: &Path, meta: &Meta) -> Result<()> {
    write_json(&dir.join("book.json"), meta)
}

pub fn save_chapter(dir: &Path, file: &str, chapter: &ChapterFile) -> Result<()> {
    write_json(&chapter_path(dir, file)?, chapter)
}

/// Rejects names that would escape the `chapters/` folder.
fn chapter_path(dir: &Path, file: &str) -> Result<PathBuf> {
    let ok = !file.is_empty()
        && file.ends_with(".json")
        && !file.contains(['/', '\\'])
        && !file.starts_with('.');
    if !ok {
        return Err(format!("invalid chapter file name {file:?}"));
    }
    Ok(dir.join("chapters").join(file))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(io_err(path))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes to a temporary file, then renames it, so a crash never leaves a half-written file.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(io_err(path))
}

/// Print layout of a project.
pub fn layout(meta: &Meta) -> book_typeset::Layout {
    book_typeset::Layout {
        style: meta.style.clone(),
        trim: meta.trim,
        bleed: meta.bleed,
        font: meta.font.clone(),
        size: meta.size,
        leading: meta.leading,
    }
}

pub fn paper(meta: &Meta) -> Result<book_typeset::Paper> {
    match meta.cover.paper.as_str() {
        "white" => Ok(book_typeset::Paper::White),
        "cream" => Ok(book_typeset::Paper::Cream),
        "color" => Ok(book_typeset::Paper::Color),
        other => Err(format!("unknown paper {other:?}")),
    }
}

/// Typesets the interior to count its pages.
pub fn page_count(meta: &Meta, chapters: &[ChapterFile]) -> Result<usize> {
    book_typeset::page_count(&book_typeset::to_typst(
        &book(meta, chapters),
        &layout(meta),
    )?)
}

/// Typst source of the cover, and the front image to pass to the compiler.
/// Needs the interior page count for the spine.
pub fn cover_source(
    dir: &Path,
    meta: &Meta,
    chapters: &[ChapterFile],
    pages: usize,
) -> Result<(String, Option<Vec<u8>>)> {
    let image = cover_image(dir, meta)?;
    let c = &meta.cover;
    let cover = book_typeset::Cover {
        spine: c
            .spine
            .unwrap_or(book_typeset::spine_width(pages, paper(meta)?)),
        blurb: c.blurb.clone(),
        background: c.background.clone(),
        ink: c.ink.clone(),
        has_image: image.is_some(),
        spine_text: pages >= book_typeset::SPINE_TEXT_MIN_PAGES,
    };
    let source = book_typeset::cover_typst(&book(meta, chapters), &layout(meta), &cover)?;
    Ok((source, image))
}

/// Book Health of a project. Typesets the interior to know its page count.
pub fn check(
    dir: &Path,
    meta: &Meta,
    chapters: &[ChapterFile],
) -> Result<Vec<book_preflight::Issue>> {
    let image = cover_image(dir, meta)?;
    let cover_image = match &image {
        Some(bytes) => Some(image_size(bytes)?),
        None => None,
    };
    let book = book(meta, chapters);
    let facts = book_preflight::Facts {
        book: &book,
        trim: meta.trim,
        bleed: meta.bleed,
        margins: book_typeset::margins(meta.trim),
        pages: page_count(meta, chapters)?,
        paper: paper(meta)?,
        cover_image,
    };
    Ok(book_preflight::check(&facts))
}

fn image_size(bytes: &[u8]) -> Result<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| format!("cover image: {e}"))
}

fn cover_image(dir: &Path, meta: &Meta) -> Result<Option<Vec<u8>>> {
    let Some(name) = &meta.cover.image else {
        return Ok(None);
    };
    let path = asset_path(dir, name)?;
    fs::read(&path).map(Some).map_err(io_err(&path))
}

/// Copies an image into `assets/` and returns its file name for `CoverSettings::image`.
pub fn import_cover_image(dir: &Path, source: &Path) -> Result<String> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| ["png", "jpg", "jpeg"].contains(&e.as_str()))
        .ok_or("the cover image must be a PNG or JPEG file")?;
    let bytes = fs::read(source).map_err(io_err(source))?;
    image_size(&bytes)?;
    // A new name for each image: other drafts and snapshots keep pointing at theirs.
    let mut stamp = now();
    let name = loop {
        let name = format!("cover-{stamp}.{ext}");
        if !asset_path(dir, &name)?.exists() {
            break name;
        }
        stamp += 1;
    };
    let path = asset_path(dir, &name)?;
    fs::create_dir_all(dir.join("assets")).map_err(io_err(dir))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    rename(&tmp, &path)?;
    remove_unused_covers(dir, &name);
    Ok(name)
}

/// Deletes cover images that no draft and no snapshot uses anymore, except `keep`,
/// which the caller is about to use. Best effort: a failure only leaves files behind.
fn remove_unused_covers(dir: &Path, keep: &str) {
    let mut metas = vec![dir.join("book.json")];
    for parent in [dir.join("drafts"), snapshots_dir(dir)] {
        if let Ok(entries) = fs::read_dir(&parent) {
            metas.extend(entries.flatten().map(|e| e.path().join("book.json")));
        }
    }
    let used: Vec<String> = metas
        .iter()
        .filter_map(|p| read_json::<Meta>(p).ok()?.cover.image)
        .chain([keep.to_string()])
        .collect();
    let Ok(entries) = fs::read_dir(dir.join("assets")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("cover-") && !used.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Rejects names that would escape the `assets/` folder.
fn asset_path(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(format!("invalid asset name {name:?}"));
    }
    Ok(dir.join("assets").join(name))
}

/// Maximum number of snapshots kept. The oldest go first.
pub const SNAPSHOT_KEEP: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    /// Unix seconds when it was taken; also its folder name.
    pub id: u64,
    /// The draft that was active when it was taken.
    pub draft: String,
    pub words: usize,
}

/// Copies the active draft into `.snapshots/<now>/`, then prunes old snapshots.
pub fn snapshot(dir: &Path) -> Result<u64> {
    let id = take_snapshot(dir)?;
    prune_snapshots(dir)?;
    Ok(id)
}

fn take_snapshot(dir: &Path) -> Result<u64> {
    let mut id = now();
    while snapshots_dir(dir).join(id.to_string()).exists() {
        id += 1;
    }
    // Copy under a hidden name, then rename: a crash never leaves a half
    // snapshot that looks complete.
    let tmp = snapshots_dir(dir).join(format!(".tmp-{id}"));
    let _ = fs::remove_dir_all(&tmp);
    copy_draft(dir, &tmp)?;
    rename(&tmp, &snapshots_dir(dir).join(id.to_string()))?;
    Ok(id)
}

fn prune_snapshots(dir: &Path) -> Result<()> {
    // Copies left by a crash in take_snapshot. No other copy runs: saves are serialized.
    if let Ok(entries) = fs::read_dir(snapshots_dir(dir)) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    let ids = snapshot_ids(dir)?;
    for old in ids.iter().take(ids.len().saturating_sub(SNAPSHOT_KEEP)) {
        let path = snapshots_dir(dir).join(old.to_string());
        fs::remove_dir_all(&path).map_err(io_err(&path))?;
    }
    Ok(())
}

/// Takes a snapshot when the newest one is older than `every` seconds.
pub fn snapshot_if_due(dir: &Path, every: u64) -> Result<()> {
    let newest = snapshot_ids(dir)?.last().copied().unwrap_or(0);
    if now().saturating_sub(newest) >= every {
        snapshot(dir)?;
    }
    Ok(())
}

/// Snapshots, newest first.
pub fn snapshots(dir: &Path) -> Result<Vec<Snapshot>> {
    let mut out = Vec::new();
    for id in snapshot_ids(dir)?.into_iter().rev() {
        // One unreadable snapshot must not hide the others.
        let Ok((meta, chapters)) = load(&snapshots_dir(dir).join(id.to_string())) else {
            continue;
        };
        let words = word_count(&book(&meta, &chapters));
        out.push(Snapshot {
            id,
            draft: meta.draft,
            words,
        });
    }
    Ok(out)
}

/// Replaces the text of the active draft with a snapshot. The current state
/// is snapshotted first, so a restore can itself be undone.
pub fn restore(dir: &Path, id: u64) -> Result<()> {
    let source = snapshots_dir(dir).join(id.to_string());
    if !source.join("book.json").is_file() {
        return Err(format!("no snapshot {id}"));
    }
    // Pruning waits for the copy: the source may be the oldest snapshot.
    take_snapshot(dir)?;
    let result = replace_with_snapshot(dir, &source);
    prune_snapshots(dir)?;
    result
}

fn replace_with_snapshot(dir: &Path, source: &Path) -> Result<()> {
    let active: Meta = read_json(&dir.join("book.json"))?;
    // Copy aside first: the active draft is only touched once the copy worked.
    let staged = dir.join(".restore");
    let _ = fs::remove_dir_all(&staged);
    copy_draft(source, &staged)?;
    // The snapshot may come from another draft: keep the active draft's name,
    // or two drafts would share one.
    let mut restored: Meta = read_json(&staged.join("book.json"))?;
    restored.draft = active.draft;
    let chapters = dir.join("chapters");
    let old = dir.join(".chapters-old");
    let _ = fs::remove_dir_all(&old);
    let moves = [
        (chapters.clone(), old.clone()),
        (staged.join("chapters"), chapters),
    ];
    move_all(&moves)?;
    if let Err(e) = save_meta(dir, &restored) {
        undo(&moves);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(&staged);
    Ok(())
}

/// Drafts: the active one first, then the others by name.
pub fn drafts(dir: &Path) -> Result<Vec<String>> {
    let meta: Meta = read_json(&dir.join("book.json"))?;
    let mut others = Vec::new();
    let path = dir.join("drafts");
    if path.is_dir() {
        for entry in fs::read_dir(&path).map_err(io_err(&path))? {
            let entry = entry.map_err(io_err(&path))?;
            if entry.path().join("book.json").is_file() {
                others.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    others.sort();
    Ok([vec![meta.draft], others].concat())
}

/// Starts a new draft from the active one. The active draft is kept as is
/// under `drafts/`, and work continues on the copy, renamed `name`.
pub fn new_draft(dir: &Path, name: &str) -> Result<()> {
    check_draft_name(dir, name)?;
    let mut meta: Meta = read_json(&dir.join("book.json"))?;
    let parked = free_parked_dir(dir, &meta.draft)?;
    if let Err(e) = copy_draft(dir, &parked) {
        let _ = fs::remove_dir_all(&parked);
        return Err(e);
    }
    meta.draft = name.to_string();
    save_meta(dir, &meta).inspect_err(|_| {
        let _ = fs::remove_dir_all(&parked);
    })
}

/// The folder where the active draft `name` gets parked. A folder left there
/// without `book.json` is an interrupted copy, not a draft: it is removed.
fn free_parked_dir(dir: &Path, name: &str) -> Result<PathBuf> {
    let parked = dir.join("drafts").join(draft_dir_name(name)?);
    if parked.join("book.json").exists() {
        return Err(format!("drafts/{name} already exists"));
    }
    if parked.exists() {
        fs::remove_dir_all(&parked).map_err(io_err(&parked))?;
    }
    Ok(parked)
}

/// Makes `name` the active draft. Only renames, undone if one fails.
pub fn switch_draft(dir: &Path, name: &str) -> Result<()> {
    let target = dir.join("drafts").join(draft_dir_name(name)?);
    if !target.join("book.json").is_file() {
        return Err(format!("no draft {name:?}"));
    }
    snapshot(dir)?;
    let meta: Meta = read_json(&dir.join("book.json"))?;
    let parked = free_parked_dir(dir, &meta.draft)?;
    fs::create_dir_all(&parked).map_err(io_err(&parked))?;
    let moves: Vec<(PathBuf, PathBuf)> = ["book.json", "chapters"]
        .iter()
        .map(|item| (dir.join(item), parked.join(item)))
        .chain(
            ["book.json", "chapters"]
                .iter()
                .map(|item| (target.join(item), dir.join(item))),
        )
        .collect();
    if let Err(e) = move_all(&moves) {
        let _ = fs::remove_dir(&parked);
        return Err(e);
    }
    fs::remove_dir(&target).map_err(io_err(&target))
}

/// Renames in order. On a failure, puts back what was already moved.
fn move_all(moves: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (i, (from, to)) in moves.iter().enumerate() {
        if let Err(e) = rename(from, to) {
            undo(&moves[..i]);
            return Err(e);
        }
    }
    Ok(())
}

fn undo(moves: &[(PathBuf, PathBuf)]) {
    for (from, to) in moves.iter().rev() {
        let _ = fs::rename(to, from);
    }
}

fn check_draft_name(dir: &Path, name: &str) -> Result<()> {
    if drafts(dir)?.iter().any(|d| d == name) {
        return Err(format!("a draft named {name:?} exists"));
    }
    draft_dir_name(name).map(|_| ())
}

/// A draft name is a folder name: no separators, not hidden, not empty.
fn draft_dir_name(name: &str) -> Result<&str> {
    let ok = !name.trim().is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.');
    if ok {
        Ok(name)
    } else {
        Err(format!("invalid draft name {name:?}"))
    }
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
}

/// Copies `book.json` and the chapter files listed in it.
fn copy_draft(from: &Path, to: &Path) -> Result<()> {
    let meta: Meta = read_json(&from.join("book.json"))?;
    fs::create_dir_all(to.join("chapters")).map_err(io_err(to))?;
    for file in &meta.chapters {
        let (src, dst) = (chapter_path(from, file)?, chapter_path(to, file)?);
        fs::copy(&src, &dst).map_err(io_err(&src))?;
    }
    write_json(&to.join("book.json"), &meta)
}

fn snapshots_dir(dir: &Path) -> PathBuf {
    dir.join(".snapshots")
}

/// Snapshot ids, oldest first. Folders that are not numbers are ignored.
fn snapshot_ids(dir: &Path) -> Result<Vec<u64>> {
    let path = snapshots_dir(dir);
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<u64> = fs::read_dir(&path)
        .map_err(io_err(&path))?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    ids.sort();
    Ok(ids)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn word_count(book: &Book) -> usize {
    let words = |inl: &[Inline]| plain(inl).split_whitespace().count();
    book.chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .map(|b| match b {
            Block::Paragraph(i) | Block::Quote(i) => words(i),
            Block::SceneBreak => 0,
        })
        .sum()
}

/// ProseMirror document to AST blocks. Unknown nodes keep their text.
pub fn blocks(doc: &Node) -> Vec<Block> {
    let mut out = Vec::new();
    for node in &doc.content {
        match node.kind.as_str() {
            "paragraph" => out.push(Block::Paragraph(inlines(&node.content))),
            "horizontalRule" => out.push(Block::SceneBreak),
            "blockquote" => out.extend(blocks(node).into_iter().map(|b| match b {
                Block::Paragraph(i) | Block::Quote(i) => Block::Quote(i),
                other => other,
            })),
            _ if !node.content.is_empty() => out.extend(blocks(node)),
            _ => {}
        }
    }
    out
}

fn inlines(nodes: &[Node]) -> Vec<Inline> {
    let mut out = Vec::new();
    for node in nodes {
        match node.kind.as_str() {
            "text" => {
                let mut inline = Inline::Text(node.text.clone().unwrap_or_default());
                // Wrap italic inside bold, so the same marks always give the same tree.
                for kind in ["italic", "bold"] {
                    if node.marks.iter().any(|m| m.kind == kind) {
                        inline = if kind == "italic" {
                            Inline::Emph(vec![inline])
                        } else {
                            Inline::Strong(vec![inline])
                        };
                    }
                }
                out.push(inline);
            }
            "footnote" => {
                let text = node
                    .attrs
                    .as_ref()
                    .and_then(|a| a["text"].as_str())
                    .unwrap_or_default();
                out.push(Inline::Footnote(vec![Inline::Text(text.into())]));
            }
            "hardBreak" => out.push(Inline::Text(" ".into())),
            _ => out.extend(inlines(&node.content)),
        }
    }
    out
}

/// AST blocks to a ProseMirror document.
pub fn doc(blocks: &[Block]) -> Node {
    let paragraph = |inl: &[Inline]| Node {
        kind: "paragraph".into(),
        content: text_nodes(inl, &[]),
        ..Node::default()
    };
    let content = blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(inl) => paragraph(inl),
            Block::Quote(inl) => Node {
                kind: "blockquote".into(),
                content: vec![paragraph(inl)],
                ..Node::default()
            },
            Block::SceneBreak => Node {
                kind: "horizontalRule".into(),
                ..Node::default()
            },
        })
        .collect();
    Node {
        kind: "doc".into(),
        content,
        ..Node::default()
    }
}

fn text_nodes(inlines: &[Inline], marks: &[&str]) -> Vec<Node> {
    let mut out = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) if t.is_empty() => {}
            Inline::Text(t) => out.push(Node {
                kind: "text".into(),
                text: Some(t.clone()),
                marks: marks
                    .iter()
                    .map(|m| Mark {
                        kind: m.to_string(),
                    })
                    .collect(),
                ..Node::default()
            }),
            Inline::Emph(c) => out.extend(text_nodes(c, &[marks, &["italic"]].concat())),
            Inline::Strong(c) => out.extend(text_nodes(c, &[marks, &["bold"]].concat())),
            // shortcut: notes are plain text in the editor, formatting inside a note is dropped.
            Inline::Footnote(c) => out.push(Node {
                kind: "footnote".into(),
                attrs: Some(serde_json::json!({ "text": plain(c) })),
                ..Node::default()
            }),
        }
    }
    out
}

fn plain(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t) => t.clone(),
            Inline::Emph(c) | Inline::Strong(c) | Inline::Footnote(c) => plain(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Inline {
        Inline::Text(t.into())
    }

    #[test]
    fn ast_survives_a_round_trip_through_the_editor_format() {
        let blocks = vec![
            Block::Paragraph(vec![
                text("Un "),
                Inline::Emph(vec![text("deux")]),
                Inline::Strong(vec![Inline::Emph(vec![text("trois")])]),
                Inline::Footnote(vec![text("note")]),
            ]),
            Block::SceneBreak,
            Block::Quote(vec![text("cité")]),
        ];
        let json = serde_json::to_string(&doc(&blocks)).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(super::blocks(&back), blocks);
    }

    #[test]
    fn reads_tiptap_output() {
        let json = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[
                {"type":"text","text":"gras italique","marks":[{"type":"bold"},{"type":"italic"}]},
                {"type":"footnote","attrs":{"text":"n"}}]},
            {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]},
                                            {"type":"paragraph","content":[{"type":"text","text":"b"}]}]},
            {"type":"paragraph"}]}"#;
        let doc: Node = serde_json::from_str(json).unwrap();
        assert_eq!(
            blocks(&doc),
            [
                Block::Paragraph(vec![
                    Inline::Strong(vec![Inline::Emph(vec![text("gras italique")])]),
                    Inline::Footnote(vec![text("n")])
                ]),
                Block::Quote(vec![text("a")]),
                Block::Quote(vec![text("b")]),
                Block::Paragraph(vec![]),
            ]
        );
    }

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("book-project-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let book = Book {
            title: "T".into(),
            author: String::new(),
            language: String::new(),
            copyright: String::new(),
            chapters: vec![Chapter {
                title: "Un".into(),
                blocks: vec![Block::Paragraph(vec![text("un deux")])],
            }],
        };
        create(&dir, &book).unwrap();
        dir
    }

    fn set_text(dir: &Path, words: &str) {
        let (meta, mut chapters) = load(dir).unwrap();
        chapters[0].doc = doc(&[Block::Paragraph(vec![text(words)])]);
        save_chapter(dir, &meta.chapters[0], &chapters[0]).unwrap();
    }

    fn words_now(dir: &Path) -> usize {
        let (meta, chapters) = load(dir).unwrap();
        word_count(&super::book(&meta, &chapters))
    }

    #[test]
    fn snapshot_then_restore() {
        let dir = temp_project("snap");
        let id = snapshot(&dir).unwrap();
        set_text(&dir, "un deux trois quatre");
        restore(&dir, id).unwrap();
        assert_eq!(words_now(&dir), 2);
        // The restore snapshotted the 4-word state first, so it can be undone.
        let list = snapshots(&dir).unwrap();
        assert_eq!(list.iter().map(|s| s.words).collect::<Vec<_>>(), [4, 2]);
        restore(&dir, list[0].id).unwrap();
        assert_eq!(words_now(&dir), 4);
        assert!(restore(&dir, 1).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_snapshots_are_pruned() {
        let dir = temp_project("prune");
        for _ in 0..SNAPSHOT_KEEP + 3 {
            snapshot(&dir).unwrap();
        }
        assert_eq!(snapshot_ids(&dir).unwrap().len(), SNAPSHOT_KEEP);
        // Restoring the oldest snapshot must not prune it before it is copied.
        // Move the snapshots to the past, so the one taken by restore is the newest.
        for (i, id) in snapshot_ids(&dir).unwrap().into_iter().enumerate() {
            let d = snapshots_dir(&dir);
            fs::rename(d.join(id.to_string()), d.join((i + 1).to_string())).unwrap();
        }
        restore(&dir, 1).unwrap();
        assert_eq!(words_now(&dir), 2);
        snapshot_if_due(&dir, 3600).unwrap();
        assert_eq!(
            snapshots(&dir).unwrap().len(),
            SNAPSHOT_KEEP,
            "a recent snapshot exists, none is due"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failures_leave_the_active_draft_usable() {
        let dir = temp_project("failures");
        // A snapshot with a missing chapter file: restore fails, the draft is intact.
        let id = snapshot(&dir).unwrap();
        fs::remove_file(
            snapshots_dir(&dir)
                .join(id.to_string())
                .join("chapters/001.json"),
        )
        .unwrap();
        assert!(restore(&dir, id).is_err());
        assert_eq!(words_now(&dir), 2);
        // A half-written snapshot folder does not break the history.
        fs::create_dir_all(snapshots_dir(&dir).join("123/chapters")).unwrap();
        assert!(snapshots(&dir).is_ok());
        // A draft without its chapters: the switch is undone.
        new_draft(&dir, "B").unwrap();
        fs::remove_dir_all(dir.join("drafts/Brouillon 1/chapters")).unwrap();
        assert!(switch_draft(&dir, "Brouillon 1").is_err());
        assert_eq!(words_now(&dir), 2);
        assert_eq!(drafts(&dir).unwrap(), ["B", "Brouillon 1"]);
        // new_draft never overwrites a parked draft with the active name...
        copy_draft(&dir, &dir.join("drafts/B")).unwrap();
        assert!(new_draft(&dir, "C").is_err());
        fs::remove_dir_all(dir.join("drafts/B")).unwrap();
        // ...but an interrupted copy there (no book.json) does not block it.
        fs::create_dir_all(dir.join("drafts/B/chapters")).unwrap();
        new_draft(&dir, "C").unwrap();
        // Crash leftovers in the history are cleaned at the next snapshot.
        fs::create_dir_all(snapshots_dir(&dir).join(".tmp-1/chapters")).unwrap();
        snapshot(&dir).unwrap();
        assert!(!snapshots_dir(&dir).join(".tmp-1").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drafts_keep_their_own_text() {
        let dir = temp_project("drafts");
        new_draft(&dir, "Brouillon 2").unwrap();
        set_text(&dir, "version deux du texte");
        assert_eq!(drafts(&dir).unwrap(), ["Brouillon 2", "Brouillon 1"]);
        switch_draft(&dir, "Brouillon 1").unwrap();
        assert_eq!(words_now(&dir), 2);
        assert_eq!(drafts(&dir).unwrap(), ["Brouillon 1", "Brouillon 2"]);
        switch_draft(&dir, "Brouillon 2").unwrap();
        assert_eq!(words_now(&dir), 4);
        assert!(new_draft(&dir, "Brouillon 1").is_err(), "names are unique");
        assert!(new_draft(&dir, "../x").is_err());
        assert!(switch_draft(&dir, "nope").is_err());

        // A snapshot taken in another draft restores its text, not its name.
        let from_b2 = snapshots(&dir)
            .unwrap()
            .into_iter()
            .find(|s| s.draft == "Brouillon 2")
            .unwrap();
        switch_draft(&dir, "Brouillon 1").unwrap();
        restore(&dir, from_b2.id).unwrap();
        assert_eq!(drafts(&dir).unwrap(), ["Brouillon 1", "Brouillon 2"]);
        switch_draft(&dir, "Brouillon 2").unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cover_and_health_of_a_project() {
        let dir = temp_project("cover");
        let (mut meta, chapters) = load(&dir).unwrap();
        let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/cover.png");
        meta.cover.image = Some(import_cover_image(&dir, &png).unwrap());
        save_meta(&dir, &meta).unwrap();
        let second = import_cover_image(&dir, &png).unwrap();
        assert_ne!(
            meta.cover.image.as_deref(),
            Some(second.as_str()),
            "each import gets its own file"
        );
        assert!(
            dir.join("assets")
                .join(meta.cover.image.as_ref().unwrap())
                .is_file(),
            "still used"
        );
        // A third import removes the second, which nothing uses.
        let third = import_cover_image(&dir, &png).unwrap();
        assert!(!dir.join("assets").join(&second).exists());
        assert!(dir.join("assets").join(&third).is_file());
        let issues = check(&dir, &meta, &chapters).unwrap();
        // One chapter of two words: too few pages, low resolution image, no author.
        for needle in ["24 pages", "ppi", "auteur"] {
            assert!(
                issues.iter().any(|i| i.message.contains(needle)),
                "{needle} missing in {issues:?}"
            );
        }
        let pages = page_count(&meta, &chapters).unwrap();
        let (src, image) = cover_source(&dir, &meta, &chapters, pages).unwrap();
        assert!(image.is_some());
        assert!(
            book_typeset::compile_pdf(&src, image.as_deref())
                .unwrap()
                .starts_with(b"%PDF-")
        );
        assert!(import_cover_image(&dir, Path::new("/etc/hostname")).is_err());
        meta.cover.image = Some("../book.json".into());
        assert!(cover_source(&dir, &meta, &chapters, pages).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_then_load_a_project() {
        let dir = std::env::temp_dir().join(format!("book-project-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let book = Book {
            title: "T".into(),
            author: "A".into(),
            language: "fr".into(),
            copyright: String::new(),
            chapters: vec![
                Chapter {
                    title: "Un".into(),
                    blocks: vec![Block::Paragraph(vec![text("x")])],
                },
                Chapter {
                    title: "Deux".into(),
                    blocks: vec![],
                },
            ],
        };
        let meta = create(&dir, &book).unwrap();
        assert_eq!(meta.chapters, ["001.json", "002.json"]);
        assert_eq!(new_chapter_file(&meta), "003.json");
        let (meta2, chapters) = load(&dir).unwrap();
        assert_eq!(meta2, meta);
        assert_eq!(super::book(&meta2, &chapters), book);
        assert!(
            create(&dir, &book).is_err(),
            "must refuse a non-empty folder"
        );
        assert!(save_chapter(&dir, "../evil.json", &chapters[0]).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
