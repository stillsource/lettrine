// Page layout shared by every style: front matter, running heads, folios,
// chapter breaks. A style only gives fonts and decorations, see styles/.

// A chapter starts on a recto. Markers around the break let the header and
// footer find the blank verso it may add, and leave that page empty.
// (A pagebreak inside `context` is ignored, so parity can't be tested there.)
// The chapter-start marker holds the chapter title, or none, for the running heads.
#let chapter-break(title: none) = {
  [#metadata(none)<chapter-end>]
  pagebreak(to: "odd", weak: true)
  [#metadata(title)<chapter-start>]
}

#let is-quiet(page) = (
  query(<chapter-end>)
    .zip(query(<chapter-start>))
    .any(((end, start)) => end.location().page() < page and page < start.location().page())
)

// Every chapter, titled or not, starts with the chapter-start marker.
#let opens-chapter(page) = (
  query(<chapter-start>).any(m => m.location().page() == page)
)

#let book(
  title: "", author: "", copyright: "", lang: "en",
  width: 5.5in, height: 8.5in, bleed: 0in,
  margin: (inside: 1in, outside: 1in, top: 1in, bottom: 1in),
  style: (:),
  body,
) = {
  set document(title: title, author: author)
  set text(lang: lang, font: style.font, size: style.size, hyphenate: true)
  set par(justify: true, first-line-indent: 1.2em, leading: style.leading, spacing: style.leading)
  // Bleed goes on the trimmed edges only, never the gutter. Typst adds it
  // outside the trim size and writes the PDF TrimBox.
  set page(
    width: width,
    height: height,
    bleed: (outside: bleed, top: bleed, bottom: bleed, inside: 0pt),
    margin: margin,
  )

  // Front matter: title page, copyright page, contents. No folios.
  align(center + horizon, {
    text(2em, title)
    v(1em)
    text(1.2em, author)
  })
  pagebreak()
  align(bottom, text(0.8em, copyright))
  pagebreak(to: "odd")
  outline(depth: 1)

  // Main matter starts on a recto at page 1.
  pagebreak(to: "odd")
  counter(page).update(1)
  // Set here so the contents title is not styled as a chapter.
  show heading.where(level: 1): it => {
    chapter-break(title: it.body)
    counter("chapter").step()
    context (style.opener)(counter("chapter").get().first(), it.body)
  }
  set page(
    header: context {
      let p = here().page()
      if not (is-quiet(p) or opens-chapter(p)) {
        set text(0.85em)
        if calc.even(p) {
          align(center, smallcaps(author))
        } else {
          let starts = query(selector(<chapter-start>).before(here()))
          if starts.len() > 0 and starts.last().value != none { align(center, smallcaps(starts.last().value)) }
        }
      }
    },
    footer: context {
      if not is-quiet(here().page()) {
        align(center, text(0.85em, counter(page).display()))
      }
    },
  )
  body
}
