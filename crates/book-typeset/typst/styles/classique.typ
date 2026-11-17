#let style = (
  font: "Libertinus Serif",
  size: 11pt,
  leading: 0.65em,
  scene-break: align(center, sym.ast.op + h(1em) + sym.ast.op + h(1em) + sym.ast.op),
  opener: (number, title) => {
    v(18%)
    align(center, text(0.9em, numbering("I", number)))
    v(0.6em)
    align(center, text(1.6em, weight: "regular", title))
    v(2.5em)
  },
)
