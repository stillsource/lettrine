//! Book AST to EPUB 3. The package is written by hand: it is a few small
//! XML files, and epub-builder 0.8.3 wrote a duplicate ID in the OPF.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use book_model::{Block, Book, Inline};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// EPUB stylesheets, by name. Names match the print styles of `book-typeset`.
pub const STYLES: &[(&str, &str)] = &[
    ("classique", include_str!("../styles/classique.css")),
    ("moderne", include_str!("../styles/moderne.css")),
];

/// One XHTML file of the book, in reading order.
struct Doc {
    href: String,
    /// Shown in the table of contents when not empty.
    toc_title: String,
    xhtml: String,
}

/// Builds the EPUB file of a book.
// shortcut: no embedded fonts, readers use their own serif/sans. Embed OFL fonts if authors ask.
pub fn to_epub(book: &Book, style: &str) -> Result<Vec<u8>, String> {
    let css = STYLES
        .iter()
        .find(|(name, _)| *name == style)
        .map(|(_, css)| *css)
        .ok_or_else(|| format!("unknown style {style:?}"))?;
    if book.title.trim().is_empty() {
        return Err("an EPUB needs a title".into());
    }
    let lang = if book.language.is_empty() {
        "en"
    } else {
        &book.language
    };

    let mut docs = vec![Doc {
        href: "title.xhtml".into(),
        toc_title: String::new(),
        xhtml: page(
            lang,
            &book.title,
            &format!(
                r#"<section class="title-page" epub:type="titlepage"><p class="title">{}</p><p>{}</p></section>"#,
                esc(&book.title),
                esc(&book.author)
            ),
        ),
    }];
    if !book.copyright.is_empty() {
        let body = format!(
            r#"<p class="copyright" epub:type="copyright-page">{}</p>"#,
            esc(&book.copyright)
        );
        docs.push(Doc {
            href: "copyright.xhtml".into(),
            toc_title: String::new(),
            xhtml: page(lang, &book.title, &body),
        });
    }
    let first_chapter = docs.len();
    for (i, chapter) in book.chapters.iter().enumerate() {
        let title = if chapter.title.is_empty() {
            &book.title
        } else {
            &chapter.title
        };
        docs.push(Doc {
            href: format!("chapter_{}.xhtml", i + 1),
            toc_title: chapter.title.clone(),
            xhtml: page(lang, title, &chapter_body(chapter)),
        });
    }
    let nav = nav(
        lang,
        &book.title,
        &docs,
        docs.get(first_chapter).map(|d| d.href.as_str()),
    );
    let opf = opf(book, lang, &docs);

    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // The mimetype entry must come first and stay uncompressed.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut files: Vec<(&str, &str)> = vec![
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/nav.xhtml", &nav),
        ("OEBPS/style.css", css),
    ];
    let paths: Vec<String> = docs.iter().map(|d| format!("OEBPS/{}", d.href)).collect();
    files.extend(
        paths
            .iter()
            .zip(&docs)
            .map(|(p, d)| (p.as_str(), d.xhtml.as_str())),
    );

    let err = |e: &dyn std::fmt::Display| e.to_string();
    zip.start_file("mimetype", stored).map_err(|e| err(&e))?;
    zip.write_all(b"application/epub+zip")
        .map_err(|e| err(&e))?;
    for (path, content) in files {
        zip.start_file(path, deflated).map_err(|e| err(&e))?;
        zip.write_all(content.as_bytes()).map_err(|e| err(&e))?;
    }
    Ok(zip.finish().map_err(|e| err(&e))?.into_inner())
}

const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"#;

fn opf(book: &Book, lang: &str, docs: &[Doc]) -> String {
    let creator = if book.author.is_empty() {
        String::new()
    } else {
        format!("<dc:creator>{}</dc:creator>", esc(&book.author))
    };
    let items: String = docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            format!(
                r#"<item id="d{i}" href="{}" media-type="application/xhtml+xml"/>"#,
                d.href
            )
        })
        .collect();
    let spine: String = (0..docs.len())
        .map(|i| format!(r#"<itemref idref="d{i}"/>"#))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="{lang}">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="book-id">urn:lettrine:{id:016x}</dc:identifier>
<dc:title>{title}</dc:title>
<dc:language>{lang}</dc:language>
{creator}
<meta property="dcterms:modified">{modified}</meta>
</metadata>
<manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="css" href="style.css" media-type="text/css"/>
{items}
</manifest>
<spine>{spine}</spine>
</package>
"#,
        lang = esc(lang),
        id = fnv1a(&format!("{}\0{}", book.title, book.author)),
        title = esc(&book.title),
        modified = utc_now(),
    )
}

fn nav(lang: &str, book_title: &str, docs: &[Doc], body_start: Option<&str>) -> String {
    let mut entries: String = docs
        .iter()
        .filter(|d| !d.toc_title.is_empty())
        .map(|d| format!(r#"<li><a href="{}">{}</a></li>"#, d.href, esc(&d.toc_title)))
        .collect();
    // The toc list must not be empty: fall back to the start of the text.
    let start = body_start.unwrap_or("title.xhtml");
    if entries.is_empty() {
        entries = format!(r#"<li><a href="{start}">{}</a></li>"#, esc(book_title));
    }
    let (heading, start_label) = if lang.starts_with("fr") {
        ("Table des matières", "Début")
    } else {
        ("Contents", "Start")
    };
    page(
        lang,
        heading,
        &format!(
            r#"<nav epub:type="toc" id="toc"><h1>{heading}</h1><ol>{entries}</ol></nav>
<nav epub:type="landmarks" hidden=""><ol><li><a epub:type="bodymatter" href="{start}">{start_label}</a></li></ol></nav>"#
        ),
    )
}

fn chapter_body(chapter: &book_model::Chapter) -> String {
    let mut body = String::from(r#"<section epub:type="chapter" role="doc-chapter">"#);
    if !chapter.title.is_empty() {
        body.push_str(&format!("<h1>{}</h1>", esc(&chapter.title)));
    }
    // Notes are numbered per chapter and listed at its end.
    let mut notes = Vec::new();
    for block in &chapter.blocks {
        match block {
            Block::Paragraph(inl) => body.push_str(&format!("<p>{}</p>", inlines(inl, &mut notes))),
            Block::Quote(inl) => body.push_str(&format!(
                "<blockquote><p>{}</p></blockquote>",
                inlines(inl, &mut notes)
            )),
            Block::SceneBreak => body.push_str(r#"<hr class="scene-break"/>"#),
        }
    }
    for (n, note) in notes.iter().enumerate() {
        let n = n + 1;
        body.push_str(&format!(
            r##"<aside class="footnote" epub:type="footnote" role="doc-footnote" id="fn{n}"><p><a href="#fnref{n}">{n}.</a> {note}</p></aside>"##
        ));
    }
    body.push_str("</section>");
    body
}

/// Stable 64-bit hash, so the book identifier survives rebuilds and Rust upgrades.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    utc(secs)
}

/// Formats Unix seconds as `YYYY-MM-DDThh:mm:ssZ` (civil-from-days, H. Hinnant).
fn utc(secs: u64) -> String {
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn page(lang: &str, title: &str, body: &str) -> String {
    let (lang, title) = (esc(lang), esc(title));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{lang}" lang="{lang}">
<head><meta charset="UTF-8"/><title>{title}</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body>{body}</body>
</html>
"#
    )
}

/// Renders inlines as XHTML. Footnote contents go to `notes`, a reference stays in the text.
fn inlines(inlines: &[Inline], notes: &mut Vec<String>) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(&esc(t)),
            Inline::Emph(c) => out.push_str(&format!("<em>{}</em>", self::inlines(c, notes))),
            Inline::Strong(c) => {
                out.push_str(&format!("<strong>{}</strong>", self::inlines(c, notes)))
            }
            Inline::Footnote(c) => {
                let content = self::inlines(c, notes);
                notes.push(content);
                let n = notes.len();
                out.push_str(&format!(
                    r##"<a epub:type="noteref" role="doc-noteref" id="fnref{n}" href="#fn{n}"><sup>{n}</sup></a>"##
                ));
            }
        }
    }
    out
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use book_model::Chapter;

    #[test]
    fn notes_are_numbered_and_text_escaped() {
        let mut notes = Vec::new();
        let html = inlines(
            &[
                Inline::Text("a < b & \"c\"".into()),
                Inline::Footnote(vec![Inline::Emph(vec![Inline::Text("n1".into())])]),
                Inline::Footnote(vec![Inline::Text("n2".into())]),
            ],
            &mut notes,
        );
        assert!(html.starts_with("a &lt; b &amp; &quot;c&quot;"));
        assert!(html.contains(r##"id="fnref2" href="#fn2""##));
        assert_eq!(notes, ["<em>n1</em>", "n2"]);
    }

    #[test]
    fn builds_a_zip_with_every_chapter() {
        let book = Book {
            title: "Essai".into(),
            author: "Moi".into(),
            language: "fr".into(),
            copyright: "© 2026".into(),
            chapters: vec![
                Chapter {
                    title: "Un".into(),
                    blocks: vec![Block::SceneBreak],
                },
                Chapter {
                    title: "Deux".into(),
                    blocks: vec![],
                },
            ],
        };
        let epub = to_epub(&book, "classique").unwrap();
        assert!(epub.starts_with(b"PK"));
        let text = String::from_utf8_lossy(&epub);
        for file in [
            "mimetype",
            "chapter_1.xhtml",
            "chapter_2.xhtml",
            "copyright.xhtml",
            "nav.xhtml",
        ] {
            assert!(text.contains(file), "{file} missing");
        }
        assert!(to_epub(&book, "nope").is_err());
        assert!(
            to_epub(
                &Book {
                    title: " ".into(),
                    ..book
                },
                "classique"
            )
            .is_err()
        );
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_825_600), "2000-02-29T12:00:00Z");
        assert_eq!(utc(1_794_830_400), "2026-11-16T12:00:00Z");
    }
}
