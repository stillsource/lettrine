//! Book Health: checks a book against print and ebook rules before export.
//!
//! Print rules are KDP's, read on 2026-11-16 on kdp.amazon.com help topics
//! GVBQ3CMEQW3W2VL6 (trim, bleed, margins, page counts) and G201953020 (cover).
//! Messages are in French, like the app.

use book_model::{Block, Book, Inline};
use book_typeset::{BLEED_IN, Margins, Paper, SPINE_TEXT_MIN_PAGES};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The printer or store will refuse the file.
    Error,
    /// The book will print, but probably not as the author wants.
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub level: Level,
    pub message: String,
}

/// What the checks need to know about a typeset book.
pub struct Facts<'a> {
    pub book: &'a Book,
    /// Trim size in inches.
    pub trim: (f64, f64),
    pub bleed: bool,
    pub margins: Margins,
    pub pages: usize,
    pub paper: Paper,
    /// Front cover image size in pixels, if there is one.
    pub cover_image: Option<(u32, u32)>,
}

/// Minimum resolution of printed images.
const MIN_PPI: f64 = 300.0;

/// KDP minimum gutter by page count: (up to this many pages, inches).
const GUTTERS: &[(usize, f64)] = &[
    (150, 0.375),
    (300, 0.5),
    (500, 0.625),
    (700, 0.75),
    (828, 0.875),
];

pub fn check(f: &Facts) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut add = |level, message: String| out.push(Issue { level, message });

    // Metadata.
    if f.book.title.trim().is_empty() {
        add(Level::Error, "Le livre n'a pas de titre.".into());
    }
    if f.book.author.trim().is_empty() {
        add(Level::Warning, "Aucun auteur n'est indiqué.".into());
    }
    if f.book.language.trim().is_empty() {
        add(
            Level::Warning,
            "La langue n'est pas indiquée : la césure et l'EPUB seront en anglais.".into(),
        );
    }
    if f.book.copyright.trim().is_empty() {
        add(Level::Info, "La page de copyright est vide.".into());
    }

    // Content.
    for (i, chapter) in f.book.chapters.iter().enumerate() {
        let n = i + 1;
        if chapter.title.trim().is_empty() && i > 0 {
            add(
                Level::Warning,
                format!(
                    "Le chapitre {n} n'a pas de titre : il manquera dans la table des matières."
                ),
            );
        }
        if !chapter.blocks.iter().any(has_text) {
            add(Level::Warning, format!("Le chapitre {n} est vide."));
        }
    }

    // Trim and page count (KDP).
    let (w, h) = f.trim;
    if !(4.0..=8.5).contains(&w) || !(6.0..=11.69).contains(&h) {
        add(
            Level::Error,
            format!(
                "Le format {w} × {h} in sort des formats KDP (4 à 8,5 in de large, 6 à 11,69 in de haut)."
            ),
        );
    }
    if f.pages < 24 {
        add(
            Level::Error,
            format!("{} pages : KDP demande au moins 24 pages.", f.pages),
        );
    }
    // shortcut: maximums of the usual trims (up to 8x10), add the per-trim table if big trims matter.
    let max = match f.paper {
        Paper::Cream => 776,
        Paper::White | Paper::Color => 828,
    };
    if f.pages > max {
        add(
            Level::Error,
            format!(
                "{} pages : KDP en accepte au plus {max} sur ce papier.",
                f.pages
            ),
        );
    }

    // Margins (KDP).
    if let Some(&(_, gutter)) = GUTTERS.iter().find(|(up_to, _)| f.pages <= *up_to)
        && f.margins.inside + 1e-9 < gutter
    {
        add(
            Level::Error,
            format!(
                "Marge intérieure de {:.3} in : KDP demande au moins {gutter} in pour {} pages. Choisissez un format plus large.",
                f.margins.inside, f.pages
            ),
        );
    }
    let min_edge = if f.bleed { 0.375 } else { 0.25 };
    let m = f.margins;
    if m.outside.min(m.top).min(m.bottom) + 1e-9 < min_edge {
        add(
            Level::Error,
            format!(
                "Les marges extérieures, haute ou basse font moins de {min_edge} in, le minimum KDP."
            ),
        );
    }

    // Cover.
    if f.pages < SPINE_TEXT_MIN_PAGES {
        add(
            Level::Info,
            format!(
                "{} pages : pas de texte sur le dos (KDP l'imprime à partir de {SPINE_TEXT_MIN_PAGES} pages).",
                f.pages
            ),
        );
    }
    if let Some((px_w, px_h)) = f.cover_image {
        // The image fills the front panel and its bleed.
        let ppi = (px_w as f64 / (w + BLEED_IN)).min(px_h as f64 / (h + 2.0 * BLEED_IN));
        if ppi < MIN_PPI {
            add(
                Level::Warning,
                format!(
                    "L'image de couverture fait {px_w} × {px_h} px, soit {ppi:.0} ppi une fois imprimée. Visez {MIN_PPI:.0} ppi : {} × {} px.",
                    ((w + BLEED_IN) * MIN_PPI).ceil(),
                    ((h + 2.0 * BLEED_IN) * MIN_PPI).ceil()
                ),
            );
        }
    }
    out
}

fn has_text(block: &Block) -> bool {
    fn inline(i: &Inline) -> bool {
        match i {
            Inline::Text(t) => !t.trim().is_empty(),
            Inline::Emph(c) | Inline::Strong(c) | Inline::Footnote(c) => c.iter().any(inline),
        }
    }
    match block {
        Block::Paragraph(i) | Block::Quote(i) => i.iter().any(inline),
        Block::SceneBreak => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use book_model::Chapter;
    use book_typeset::margins;

    fn book() -> Book {
        Book {
            title: "T".into(),
            author: "A".into(),
            language: "fr".into(),
            copyright: "©".into(),
            chapters: vec![Chapter {
                title: "Un".into(),
                blocks: vec![Block::Paragraph(vec![Inline::Text("x".into())])],
            }],
        }
    }

    fn facts(book: &Book, trim: (f64, f64), pages: usize) -> Facts<'_> {
        Facts {
            book,
            trim,
            bleed: false,
            margins: margins(trim),
            pages,
            paper: Paper::White,
            cover_image: None,
        }
    }

    fn errors(f: &Facts) -> Vec<String> {
        check(f)
            .into_iter()
            .filter(|i| i.level == Level::Error)
            .map(|i| i.message)
            .collect()
    }

    #[test]
    fn a_normal_book_passes() {
        let b = book();
        assert!(errors(&facts(&b, (6.0, 9.0), 200)).is_empty());
        assert!(check(&facts(&b, (6.0, 9.0), 200)).is_empty());
    }

    #[test]
    fn page_count_limits() {
        let b = book();
        assert_eq!(errors(&facts(&b, (6.0, 9.0), 20)).len(), 1);
        let mut f = facts(&b, (6.0, 9.0), 800);
        f.paper = Paper::Cream;
        assert!(errors(&f)[0].contains("776"));
    }

    #[test]
    fn gutter_follows_page_count() {
        let b = book();
        // 5x8 gives a 0.75 in gutter: enough up to 700 pages, not above.
        assert!(errors(&facts(&b, (5.0, 8.0), 700)).is_empty());
        assert!(
            errors(&facts(&b, (5.0, 8.0), 701))
                .iter()
                .any(|e| e.contains("0.875"))
        );
    }

    #[test]
    fn trim_outside_kdp_range() {
        let b = book();
        assert!(
            errors(&facts(&b, (3.5, 8.0), 100))
                .iter()
                .any(|e| e.contains("format"))
        );
    }

    #[test]
    fn low_resolution_cover_image() {
        let b = book();
        let mut f = facts(&b, (6.0, 9.0), 200);
        f.cover_image = Some((600, 900));
        assert!(
            check(&f)
                .iter()
                .any(|i| i.level == Level::Warning && i.message.contains("ppi"))
        );
        f.cover_image = Some((1838, 2775));
        assert!(check(&f).is_empty());
    }

    #[test]
    fn missing_metadata_and_empty_chapters() {
        let mut b = book();
        b.title = " ".into();
        b.chapters.push(Chapter {
            title: String::new(),
            blocks: vec![Block::SceneBreak],
        });
        let issues = check(&facts(&b, (6.0, 9.0), 200));
        assert!(
            issues
                .iter()
                .any(|i| i.level == Level::Error && i.message.contains("titre"))
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("chapitre 2 n'a pas de titre"))
        );
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("chapitre 2 est vide"))
        );
    }
}
