//! Book AST to Typst source, then to PDF with the embedded Typst compiler.

use book_model::{Block, Book, Inline};
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Dict, Value};
use typst::syntax::{FileId, Source};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

const TEMPLATE: &str = include_str!("../typst/book.typ");

/// Book styles, by name. A style is a Typst file that defines `style`.
pub const STYLES: &[(&str, &str)] = &[
    ("classique", include_str!("../typst/styles/classique.typ")),
    ("moderne", include_str!("../typst/styles/moderne.typ")),
];

/// Common trim sizes in inches (width, height).
pub const TRIMS: &[(&str, f64, f64)] = &[
    ("5x8", 5.0, 8.0),
    ("5.25x8", 5.25, 8.0),
    ("5.5x8.5", 5.5, 8.5),
    ("6x9", 6.0, 9.0),
    ("a5", 5.83, 8.27),
];

/// Bleed on the trimmed edges, in inches. KDP and IngramSpark both ask for 0.125 in
/// on the top, bottom and outside edges, none on the gutter.
pub const BLEED_IN: f64 = 0.125;

/// Page margins in inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub inside: f64,
    pub outside: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Margins for a trim size, as a share of the page. They stay above the KDP
/// minimums for usual trims and page counts; `book-preflight` checks the rest.
pub fn margins(trim: (f64, f64)) -> Margins {
    let (w, h) = trim;
    Margins {
        inside: 0.15 * w,
        outside: 0.11 * w,
        top: 0.09 * h,
        bottom: 0.1 * h,
    }
}

/// Interior paper. Sets the thickness of a page, so the spine width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    White,
    Cream,
    Color,
}

impl Paper {
    /// Inches per page, from KDP's cover guide (help topic G201953020, read 2026-11-16).
    /// Color is the premium color value. IngramSpark publishes no table, its
    /// cover template generator gives the spine width.
    pub fn inch_per_page(self) -> f64 {
        match self {
            Paper::White => 0.002252,
            Paper::Cream => 0.0025,
            Paper::Color => 0.002347,
        }
    }
}

/// Spine width in inches for a page count, by KDP's formula.
pub fn spine_width(pages: usize, paper: Paper) -> f64 {
    pages as f64 * paper.inch_per_page()
}

/// KDP prints spine text only on books with more than 79 pages.
pub const SPINE_TEXT_MIN_PAGES: usize = 80;

/// How the book is laid out, independent of its content.
#[derive(Debug, Clone)]
pub struct Layout {
    pub style: String,
    /// Trim size in inches (width, height).
    pub trim: (f64, f64),
    pub bleed: bool,
    /// Overrides of the style. `None` keeps the style's value.
    pub font: Option<String>,
    /// Body text size in points.
    pub size: Option<f64>,
    /// Space between lines, in em.
    pub leading: Option<f64>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            style: "classique".into(),
            trim: (5.5, 8.5),
            bleed: false,
            font: None,
            size: None,
            leading: None,
        }
    }
}

/// Builds the Typst source of a book.
pub fn to_typst(book: &Book, layout: &Layout) -> Result<String, String> {
    let style = STYLES
        .iter()
        .find(|(name, _)| *name == layout.style)
        .map(|(_, src)| *src)
        .ok_or_else(|| format!("unknown style {:?}", layout.style))?;
    let lang = typst_lang(&book.language);
    let bleed = if layout.bleed { BLEED_IN } else { 0.0 };
    let overrides = overrides(layout)?;
    let mut out = format!(
        "{style}\n{TEMPLATE}\n#show: book.with(title: {}, author: {}, copyright: {}, lang: {}, \
         width: {}in, height: {}in, bleed: {bleed}in, margin: {}, style: style + {overrides})\n",
        lit(&book.title),
        lit(&book.author),
        lit(&book.copyright),
        lit(&lang),
        layout.trim.0,
        layout.trim.1,
        typst_margins(margins(layout.trim)),
    );
    for chapter in &book.chapters {
        // The heading show rule breaks to a recto. Without a title, break here.
        if chapter.title.is_empty() {
            out.push_str("\n#chapter-break()\n");
        } else {
            out.push_str(&format!("\n#heading(level: 1)[#{}]\n", lit(&chapter.title)));
        }
        for block in &chapter.blocks {
            out.push('\n');
            match block {
                Block::Paragraph(inl) => inlines(&mut out, inl),
                Block::Quote(inl) => {
                    out.push_str("#quote(block: true)[");
                    inlines(&mut out, inl);
                    out.push(']');
                }
                Block::SceneBreak => out.push_str("#style.scene-break"),
            }
            out.push('\n');
        }
    }
    Ok(out)
}

/// Typst takes an ISO 639 code only. The book holds a BCP 47 tag like `fr-FR`:
/// keep its language part. Anything else, like a half-typed value, falls back to English.
fn typst_lang(tag: &str) -> String {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_lowercase());
    if valid { primary } else { "en".into() }
}

fn typst_margins(m: Margins) -> String {
    format!(
        "(inside: {}in, outside: {}in, top: {}in, bottom: {}in)",
        m.inside, m.outside, m.top, m.bottom
    )
}

/// What goes on the cover besides the book's title and author.
#[derive(Debug, Clone)]
pub struct Cover {
    /// Spine width in inches.
    pub spine: f64,
    /// Back cover text.
    pub blurb: String,
    /// `#rrggbb` colors.
    pub background: String,
    pub ink: String,
    /// Front image, passed to `compile_*` as bytes. It fills the front panel and its bleed.
    pub has_image: bool,
    pub spine_text: bool,
}

/// Builds the Typst source of the full cover: back, spine and front on one page,
/// with bleed on all four sides as KDP and IngramSpark ask.
pub fn cover_typst(book: &Book, layout: &Layout, cover: &Cover) -> Result<String, String> {
    for color in [&cover.background, &cover.ink] {
        let hex = color.strip_prefix('#').unwrap_or_default();
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("color {color:?} is not #rrggbb"));
        }
    }
    if !(0.0..=4.0).contains(&cover.spine) {
        return Err(format!("spine width {}in is out of 0 to 4 in", cover.spine));
    }
    let style = STYLES
        .iter()
        .find(|(name, _)| *name == layout.style)
        .map(|(_, src)| *src)
        .ok_or_else(|| format!("unknown style {:?}", layout.style))?;
    let front = if cover.has_image {
        "place(top + left, dx: bleed + trim-w + spine, image(sys.inputs.image, width: trim-w + bleed, height: trim-h + 2 * bleed, fit: \"cover\"))"
    } else {
        "none"
    };
    Ok(format!(
        r#"{style}
#let (trim-w, trim-h, spine, bleed) = ({w}in, {h}in, {spine}in, {BLEED_IN}in)
#let ink = rgb({ink})
// Text stays this far inside the trim and the spine folds.
#let safe = 0.375in
#set page(
  width: 2 * trim-w + spine, height: trim-h, bleed: bleed, margin: 0pt,
  background: {{
    rect(width: 100%, height: 100%, fill: rgb({background}))
    {front}
  }},
)
#set text(font: (style + {overrides}).font, fill: ink, lang: {lang})
#set par(justify: false)
// Back: the blurb, clear of the bottom right corner where the printer puts the barcode.
#place(top + left, dx: safe, dy: safe + 0.25in,
  box(width: trim-w - 2 * safe, height: trim-h - 2 * safe - 1.5in, text(size: 11pt, {blurb})))
#if {spine_text} {{
  place(top + left, dx: trim-w, box(width: spine, height: trim-h, align(center + horizon,
    rotate(90deg, reflow: true, text(size: calc.min(10pt, spine * 0.45), [#{title} #h(1.5em) #{author}])))))
}}
#if not {has_image} {{
  place(top + left, dx: trim-w + spine + safe, dy: safe,
    box(width: trim-w - 2 * safe, height: trim-h - 2 * safe, {{
      v(22%)
      align(center, text(size: 30pt, {title}))
      v(1fr)
      align(center, text(size: 16pt, {author}))
      v(8%)
    }}))
}}
"#,
        w = layout.trim.0,
        h = layout.trim.1,
        spine = cover.spine,
        ink = lit(&cover.ink),
        background = lit(&cover.background),
        overrides = overrides(layout)?,
        lang = lit(&typst_lang(&book.language)),
        blurb = lit(&cover.blurb),
        spine_text = cover.spine_text,
        title = lit(&book.title),
        author = lit(&book.author),
        has_image = cover.has_image,
    ))
}

/// The layout's style overrides as a Typst dictionary, merged into the style.
fn overrides(layout: &Layout) -> Result<String, String> {
    let mut entries = Vec::new();
    if let Some(font) = &layout.font {
        entries.push(format!("font: {}", lit(font)));
    }
    if let Some(size) = layout.size {
        if !(6.0..=24.0).contains(&size) {
            return Err(format!("text size {size}pt is out of 6 to 24 pt"));
        }
        entries.push(format!("size: {size}pt"));
    }
    if let Some(leading) = layout.leading {
        if !(0.2..=2.0).contains(&leading) {
            return Err(format!("leading {leading}em is out of 0.2 to 2 em"));
        }
        entries.push(format!("leading: {leading}em"));
    }
    Ok(if entries.is_empty() {
        "(:)".into()
    } else {
        format!("({})", entries.join(", "))
    })
}

/// Font families bundled with the app, usable in `Layout::font`.
pub fn font_families() -> Vec<String> {
    let mut names: Vec<String> = fonts()
        .1
        .families()
        .map(|(name, _)| name.to_string())
        .collect();
    names.dedup();
    names
}

fn inlines(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(&format!("#{}", lit(t))),
            Inline::Emph(c) => wrap(out, "emph", c),
            Inline::Strong(c) => wrap(out, "strong", c),
            Inline::Footnote(c) => wrap(out, "footnote", c),
        }
    }
}

fn wrap(out: &mut String, func: &str, content: &[Inline]) {
    out.push_str(&format!("#{func}["));
    inlines(out, content);
    out.push(']');
}

/// Text as a Typst string literal. Only `\` and `"` need escaping, so author
/// text can never be read as markup. Prefix with `#` to use it in markup.
fn lit(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Compiles Typst source to PDF bytes. Errors are returned as readable messages.
/// `image` is exposed to the source as `sys.inputs.image`, see [`cover_typst`].
pub fn compile_pdf(source: &str, image: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let document = compile(source, image)?;
    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|e| messages(&e))
}

/// Compiles Typst source to one SVG per page, for the preview.
pub fn compile_svg(source: &str, image: Option<&[u8]>) -> Result<Vec<String>, String> {
    let document = compile(source, image)?;
    let options = typst_svg::SvgOptions::default();
    Ok(document
        .pages()
        .iter()
        .map(|page| typst_svg::svg(page, &options))
        .collect())
}

/// Number of pages of the typeset book.
pub fn page_count(source: &str) -> Result<usize, String> {
    Ok(compile(source, None)?.pages().len())
}

fn compile(source: &str, image: Option<&[u8]>) -> Result<PagedDocument, String> {
    let document = typst::compile(&BookWorld::new(source, image))
        .output
        .map_err(|e| messages(&e));
    // Typst memoizes across compiles. The app compiles on every edit: drop entries
    // unused for 10 compiles, as typst-cli does, or memory grows for the whole session.
    typst::comemo::evict(10);
    document
}

/// The bundled fonts, parsed once per process.
fn fonts() -> &'static (Vec<Font>, LazyHash<FontBook>) {
    static FONTS: std::sync::OnceLock<(Vec<Font>, LazyHash<FontBook>)> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let fonts: Vec<Font> = typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect();
        let book = LazyHash::new(FontBook::from_fonts(&fonts));
        (fonts, book)
    })
}

fn messages(diags: &[SourceDiagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A sandbox with one in-memory source and the fonts bundled with Typst.
/// It reads nothing from disk.
struct BookWorld {
    library: LazyHash<Library>,
    main: Source,
}

impl BookWorld {
    fn new(source: &str, image: Option<&[u8]>) -> Self {
        let mut inputs = Dict::new();
        if let Some(bytes) = image {
            inputs.insert("image".into(), Value::Bytes(Bytes::new(bytes.to_vec())));
        }
        Self {
            library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
            main: Source::detached(source),
        }
    }
}

impl World for BookWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &fonts().1
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        fonts().0.get(index).cloned()
    }
    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use book_model::Chapter;

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(text.into())])
    }

    fn book() -> Book {
        let chapter = |title: &str| Chapter {
            title: title.into(),
            blocks: vec![para("Un paragraphe.")],
        };
        Book {
            title: "Essai".into(),
            author: "Moi".into(),
            language: "fr".into(),
            copyright: "© 2026 Moi".into(),
            chapters: vec![
                Chapter {
                    title: "Un".into(),
                    blocks: vec![
                        Block::Paragraph(vec![
                            Inline::Text("= #pas du *markup* \\ ".into()),
                            Inline::Footnote(vec![Inline::Text("note".into())]),
                        ]),
                        Block::SceneBreak,
                        Block::Quote(vec![Inline::Emph(vec![Inline::Text("cité".into())])]),
                    ],
                },
                chapter("Deux"),
                chapter("Trois"),
            ],
        }
    }

    #[test]
    fn author_text_is_never_markup() {
        assert_eq!(lit(r#"#set "x" \ = *"#), r##""#set \"x\" \\ = *""##);
    }

    #[test]
    fn every_style_compiles_to_pdf() {
        for (name, _) in STYLES {
            let layout = Layout {
                style: name.to_string(),
                ..Layout::default()
            };
            let pdf = compile_pdf(&to_typst(&book(), &layout).unwrap(), None).unwrap();
            assert!(pdf.starts_with(b"%PDF-"), "style {name}");
        }
    }

    #[test]
    fn overrides_reach_the_page() {
        let layout = Layout {
            font: Some("New Computer Modern".into()),
            size: Some(14.0),
            leading: Some(1.0),
            ..Layout::default()
        };
        let src = to_typst(&book(), &layout).unwrap();
        assert!(
            src.contains(
                r#"style: style + (font: "New Computer Modern", size: 14pt, leading: 1em)"#
            )
        );
        // Bigger text and leading give more pages than the default.
        let pages = |l: &Layout| {
            compile(&to_typst(&book(), l).unwrap(), None)
                .unwrap()
                .pages()
                .len()
        };
        let long = Book {
            chapters: vec![Chapter {
                title: "X".into(),
                blocks: vec![para(&"mot ".repeat(400))],
            }],
            ..book()
        };
        let count = |l: &Layout| {
            compile(&to_typst(&long, l).unwrap(), None)
                .unwrap()
                .pages()
                .len()
        };
        assert!(count(&layout) > count(&Layout::default()));
        assert_eq!(
            pages(&layout),
            pages(&Layout::default()),
            "short chapters still fit one page each"
        );
        assert!(
            to_typst(
                &book(),
                &Layout {
                    size: Some(f64::NAN),
                    ..Layout::default()
                }
            )
            .is_err()
        );
        assert!(font_families().iter().any(|f| f == "Libertinus Serif"));
    }

    #[test]
    fn language_tags_are_reduced_to_what_typst_takes() {
        assert_eq!(typst_lang("fr-FR"), "fr");
        assert_eq!(typst_lang("pt_BR"), "pt");
        for bad in ["", "f", "français", "12"] {
            assert_eq!(typst_lang(bad), "en", "{bad:?}");
        }
        let b = Book {
            language: "fr-FR".into(),
            ..book()
        };
        assert!(compile(&to_typst(&b, &Layout::default()).unwrap(), None).is_ok());
    }

    #[test]
    fn unknown_style_is_an_error() {
        let layout = Layout {
            style: "nope".into(),
            ..Layout::default()
        };
        assert!(to_typst(&book(), &layout).is_err());
    }

    #[test]
    fn chapters_open_on_recto_and_blank_versos_stay_empty() {
        let doc = compile(&to_typst(&book(), &Layout::default()).unwrap(), None).unwrap();
        // Front matter: title (1), copyright (2), contents (3), blank (4).
        // Then one page per chapter, each on a recto, with an empty verso between.
        assert_eq!(doc.pages().len(), 9);
        for blank in [3, 5, 7] {
            assert!(
                !has_ink(&doc.pages()[blank].frame),
                "page {} must be empty",
                blank + 1
            );
        }
    }

    fn collect_text(frame: &typst::layout::Frame, out: &mut String) {
        use typst::layout::FrameItem;
        for (_, item) in frame.items() {
            match item {
                FrameItem::Group(g) => collect_text(&g.frame, out),
                FrameItem::Text(t) => out.push_str(t.text.as_str()),
                _ => {}
            }
        }
    }

    #[test]
    fn an_untitled_chapter_has_no_running_head_from_the_previous_one() {
        let mut b = book();
        b.chapters.truncate(2);
        b.chapters[1] = Chapter {
            title: String::new(),
            blocks: vec![para(&"mot ".repeat(2000))],
        };
        let doc = compile(&to_typst(&b, &Layout::default()).unwrap(), None).unwrap();
        // Pages: title, copyright, contents, blank, chapter "Un", blank, then chapter 2.
        assert!(doc.pages().len() > 8, "chapter 2 must span several pages");
        for (i, page) in doc.pages().iter().enumerate().skip(6) {
            let mut text = String::new();
            collect_text(&page.frame, &mut text);
            assert!(
                !text.contains("Un"),
                "page {} shows chapter 1's title",
                i + 1
            );
        }
    }

    /// True when the frame draws something. Tags and links are invisible.
    fn has_ink(frame: &typst::layout::Frame) -> bool {
        use typst::layout::FrameItem;
        frame.items().any(|(_, item)| match item {
            FrameItem::Group(g) => has_ink(&g.frame),
            FrameItem::Text(_) | FrameItem::Shape(..) | FrameItem::Image(..) => true,
            FrameItem::Link(..) | FrameItem::Tag(_) => false,
        })
    }

    #[test]
    fn untitled_chapters_also_start_on_a_recto() {
        let mut b = book();
        b.chapters[1].title.clear();
        let doc = compile(&to_typst(&b, &Layout::default()).unwrap(), None).unwrap();
        assert_eq!(doc.pages().len(), 9, "same pages as with a title");
        assert!(
            !has_ink(&doc.pages()[5].frame),
            "the verso before it stays blank"
        );
    }

    #[test]
    fn bleed_is_added_around_trim() {
        let layout = Layout {
            trim: (6.0, 9.0),
            bleed: true,
            ..Layout::default()
        };
        let doc = compile(&to_typst(&book(), &layout).unwrap(), None).unwrap();
        let page = &doc.pages()[0];
        assert!((page.frame.width().to_inches() - 6.0).abs() < 1e-6);
        assert!((page.bleed.top.to_inches() - BLEED_IN).abs() < 1e-6);
        assert_eq!(
            page.bleed.left.to_inches() + page.bleed.right.to_inches(),
            BLEED_IN,
            "no bleed in the gutter"
        );
    }

    fn cover(spine: f64, has_image: bool) -> Cover {
        Cover {
            spine,
            blurb: "Quatrième de couverture.".into(),
            background: "#1d3557".into(),
            ink: "#f1faee".into(),
            has_image,
            spine_text: true,
        }
    }

    #[test]
    fn cover_is_back_spine_front_with_bleed_all_around() {
        let layout = Layout {
            trim: (6.0, 9.0),
            ..Layout::default()
        };
        let spine = spine_width(300, Paper::White);
        assert!((spine - 0.6756).abs() < 1e-9);
        let src = cover_typst(&book(), &layout, &cover(spine, false)).unwrap();
        let doc = compile(&src, None).unwrap();
        let page = &doc.pages()[0];
        assert_eq!(doc.pages().len(), 1);
        assert!((page.frame.width().to_inches() - (12.0 + spine)).abs() < 1e-6);
        for side in [
            page.bleed.left,
            page.bleed.right,
            page.bleed.top,
            page.bleed.bottom,
        ] {
            assert!((side.to_inches() - BLEED_IN).abs() < 1e-6);
        }
        let image = include_bytes!("../../../fixtures/cover.png");
        let src = cover_typst(&book(), &layout, &cover(spine, true)).unwrap();
        assert!(
            compile_pdf(&src, Some(image))
                .unwrap()
                .starts_with(b"%PDF-")
        );
    }

    #[test]
    fn cover_rejects_bad_input() {
        let bad_color = Cover {
            background: "red\") #evil".into(),
            ..cover(0.5, false)
        };
        assert!(cover_typst(&book(), &Layout::default(), &bad_color).is_err());
        assert!(cover_typst(&book(), &Layout::default(), &cover(f64::NAN, false)).is_err());
    }
}
