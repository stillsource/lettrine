# lettrine

Open source book formatting for authors. Manuscript in, print-ready PDF and EPUB out.
Local first: a book is a folder on your disk.

Status: v1 scope done. Desktop app (editor, live preview, drafts, history, cover, Book Health), Markdown import, print PDF, cover PDF and EPUB 3.

## Desktop app

Needs Node 20+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
(on Linux: webkit2gtk 4.1).

```sh
cd app
npm install
npm run tauri dev                  # or: npx tauri build
```

`lettrine <project-dir>` opens a project on start.

Installers: `npx tauri build` makes the ones of the current OS. Pushing a `v*` tag builds
Linux, macOS and Windows installers in CI (`.github/workflows/release.yml`), unsigned,
on a draft release.

A project is a folder: `book.json` (metadata, style, trim, chapter order) and one
`chapters/NNN.json` per chapter, in the editor's ProseMirror JSON. Saves go through
a temporary file and a rename. See `crates/book-project`.

- **Drafts**: the active draft is at the root, the others in `drafts/<name>/`, same layout.
  A new draft keeps the current one as is and continues on a copy.
- **History**: `.snapshots/<unix seconds>/` holds copies of the active draft, taken at most
  every 10 minutes of writing and before each restore or draft switch. The 50 newest are kept.
  Restoring replaces the text of the active draft and keeps its name.
- **Style overrides**: font, text size and leading go on top of the style, for print only.
  Ebook readers choose their own.
- **Cover**: back, spine and front on one PDF page, 0.125 in bleed on all sides. The spine
  width comes from KDP's formula for the paper (white, cream, color) and the page count,
  or from a value you enter (IngramSpark gives it in their cover template). Spine text
  only from 80 pages. The front image goes in `assets/`, shared by all drafts.
- **Book Health** (`book-preflight`): KDP trim range, page count limits, gutter by page count,
  edge margins, cover image resolution (300 ppi), missing metadata, empty or untitled chapters.

## Command line

```sh
cargo run -p book -- render fixtures/demo.md -o demo.pdf --epub demo.epub \
  --title "Le phare" --author "Anonyme" --lang fr --copyright "© 2026 Anonyme" \
  --style moderne --trim 6x9 --bleed
```

- `-o` writes the print PDF, `--epub` the ebook. Give one or both.
- `--style`: `classique` (default) or `moderne`.
- `--trim`: `5x8`, `5.25x8`, `5.5x8.5` (default), `6x9`, `a5`, or any `WxH` in inches.
- `--bleed`: adds 0.125 in on the outside, top and bottom edges, and writes the PDF TrimBox.
- `--typst out.typ`: also writes the generated Typst source.

`book import manuscript.md my-book/ --title "Le phare"` creates a project folder for the app.
`book check my-book/` prints Book Health and fails on blocking issues.
`book cover my-book/ -o cover.pdf` writes the full cover.

## Styles

A style is a Typst file in `crates/book-typeset/typst/styles/` that defines a `style`
dictionary: font, size, leading, scene break and chapter opener. Page layout (front
matter, running heads, folios, blank versos) is shared, in `typst/book.typ`.
Each style also has an EPUB stylesheet in `crates/book-epub/styles/`.
To add a style, add both files and one line to `STYLES` in `book-typeset` and `book-epub`.
A test checks that both lists match.

## Layout

| Crate | Role |
|---|---|
| `book-model` | Book AST. Every importer produces it, every renderer reads it. |
| `book-import` | Markdown to AST. A `#` heading starts a chapter, `---` is a scene break. |
| `book-typeset` | AST to Typst source, then PDF with Typst embedded as a library. Holds styles and trim presets. |
| `book-project` | Project folder on disk, and conversion between the editor JSON and the AST. |
| `book-preflight` | Book Health rules, with their sources. |
| `book-epub` | AST to EPUB 3. The package files are written by hand, see the crate doc. |
| `book-cli` | The `book` command. |
| `app/` | Tauri 2 + React + TipTap desktop app. All disk and rendering work goes through the crates. |

PDF typesetting uses [Typst](https://typst.app) 0.15. It has no PDF/X output and no CMYK
output intent, so print PDFs are RGB. IngramSpark's guide asks for CMYK images in color
interiors: check with your printer before a color book.

Print values (bleed, margins, spine) come from KDP's help pages and IngramSpark's file
creation guide, read on 2026-11-16. Sources are cited in the code next to each value.

CI runs [EPUBCheck](https://github.com/w3c/epubcheck) on the demo book.

## License

AGPL-3.0-or-later, see [LICENSE](LICENSE). If you run a modified version as a service,
you must publish its source.
