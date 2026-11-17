//! The `book` command, see USAGE.

use anyhow::{Context, Result, bail};
use book_model::Book;
use book_typeset::{Layout, STYLES, TRIMS};

const USAGE: &str = "usage: book import <manuscript.md> <project-dir> [--title T]
       book check <project-dir>
       book cover <project-dir> -o cover.pdf
       book render <manuscript.md> [-o out.pdf] [--epub out.epub]
  [--title T] [--author A] [--lang fr] [--copyright TEXT]
  [--style NAME] [--trim NAME|WxH] [--bleed] [--typst out.typ]";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("render") => {}
        Some("import") => return import(args.collect()),
        Some("check") => return check(args.collect()),
        Some("cover") => return cover(args.collect()),
        _ => bail!("{USAGE}"),
    }
    let (mut input, mut output, mut epub_out, mut typst_out) = (None, None, None, None);
    let mut book = Book {
        title: String::new(),
        author: String::new(),
        language: String::new(),
        copyright: String::new(),
        chapters: Vec::new(),
    };
    let mut layout = Layout::default();
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-o" => output = Some(value()?),
            "--epub" => epub_out = Some(value()?),
            "--title" => book.title = value()?,
            "--author" => book.author = value()?,
            "--lang" => book.language = value()?,
            "--copyright" => book.copyright = value()?,
            "--style" => layout.style = value()?,
            "--trim" => layout.trim = trim(&value()?)?,
            "--bleed" => layout.bleed = true,
            "--typst" => typst_out = Some(value()?),
            _ if input.is_none() => input = Some(arg),
            _ => bail!("unexpected argument: {arg}\n{USAGE}"),
        }
    }
    let input = input.context("missing manuscript path")?;
    if output.is_none() && epub_out.is_none() && typst_out.is_none() {
        bail!("nothing to write: give -o, --epub or --typst\n{USAGE}");
    }

    let src = std::fs::read_to_string(&input).with_context(|| format!("reading {input}"))?;
    book.chapters = book_import::markdown(&src);
    if let Some(path) = epub_out {
        let epub = book_epub::to_epub(&book, &layout.style)
            .map_err(anyhow::Error::msg)
            .context("EPUB failed")?;
        std::fs::write(&path, epub).with_context(|| format!("writing {path}"))?;
    }
    let styles: Vec<_> = STYLES.iter().map(|(n, _)| *n).collect();
    let typst = book_typeset::to_typst(&book, &layout)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("styles: {}", styles.join(", ")))?;
    if let Some(path) = typst_out {
        std::fs::write(&path, &typst).with_context(|| format!("writing {path}"))?;
    }
    if let Some(path) = output {
        let pdf = book_typeset::compile_pdf(&typst, None)
            .map_err(anyhow::Error::msg)
            .context("typesetting failed")?;
        std::fs::write(&path, pdf).with_context(|| format!("writing {path}"))?;
    }
    Ok(())
}

/// Creates a project folder, the format the desktop app opens, from a manuscript.
fn import(args: Vec<String>) -> Result<()> {
    let (input, dir, title) = match args.as_slice() {
        [input, dir] => (input, dir, None),
        [input, dir, flag, title] if flag == "--title" => (input, dir, Some(title.clone())),
        _ => bail!("{USAGE}"),
    };
    let src = std::fs::read_to_string(input).with_context(|| format!("reading {input}"))?;
    let dir = std::path::Path::new(dir);
    let title = title.unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let book = Book {
        title,
        author: String::new(),
        language: String::new(),
        copyright: String::new(),
        chapters: book_import::markdown(&src),
    };
    book_project::create(dir, &book).map_err(anyhow::Error::msg)?;
    Ok(())
}

/// Prints Book Health. Fails when a printer would refuse the book, for CI.
fn check(args: Vec<String>) -> Result<()> {
    let [dir] = args.as_slice() else {
        bail!("{USAGE}")
    };
    let dir = std::path::Path::new(dir);
    let (meta, chapters) = book_project::load(dir).map_err(anyhow::Error::msg)?;
    let issues = book_project::check(dir, &meta, &chapters).map_err(anyhow::Error::msg)?;
    for issue in &issues {
        println!(
            "{:<8} {}",
            format!("{:?}", issue.level).to_lowercase(),
            issue.message
        );
    }
    let errors = issues
        .iter()
        .filter(|i| i.level == book_preflight::Level::Error)
        .count();
    if errors > 0 {
        bail!("{errors} error(s)");
    }
    Ok(())
}

fn cover(args: Vec<String>) -> Result<()> {
    let [dir, flag, out] = args.as_slice() else {
        bail!("{USAGE}")
    };
    if flag != "-o" {
        bail!("{USAGE}");
    }
    let dir = std::path::Path::new(dir);
    let (meta, chapters) = book_project::load(dir).map_err(anyhow::Error::msg)?;
    let pages = book_project::page_count(&meta, &chapters).map_err(anyhow::Error::msg)?;
    let (source, image) =
        book_project::cover_source(dir, &meta, &chapters, pages).map_err(anyhow::Error::msg)?;
    let pdf = book_typeset::compile_pdf(&source, image.as_deref()).map_err(anyhow::Error::msg)?;
    std::fs::write(out, pdf).with_context(|| format!("writing {out}"))?;
    Ok(())
}

/// A preset name from TRIMS, or a custom size in inches like `5.5x8.5`.
fn trim(value: &str) -> Result<(f64, f64)> {
    if let Some((_, w, h)) = TRIMS.iter().find(|(name, ..)| *name == value) {
        return Ok((*w, *h));
    }
    let parse = || -> Option<(f64, f64)> {
        let (w, h) = value.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    };
    match parse() {
        Some((w, h)) if w > 0.0 && h > 0.0 => Ok((w, h)),
        _ => {
            let names: Vec<_> = TRIMS.iter().map(|(n, ..)| *n).collect();
            bail!(
                "bad trim {value:?}: use WxH in inches or one of {}",
                names.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn print_and_ebook_offer_the_same_styles() {
        let names = |s: &[(&str, &str)]| s.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>();
        assert_eq!(names(book_typeset::STYLES), names(book_epub::STYLES));
    }
}
