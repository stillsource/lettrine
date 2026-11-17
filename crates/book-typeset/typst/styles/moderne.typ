#let style = (
  font: "New Computer Modern",
  size: 10.5pt,
  leading: 0.7em,
  scene-break: align(center, line(length: 25%, stroke: 0.4pt)),
  opener: (number, title) => {
    v(12%)
    text(3em, weight: "bold", str(number))
    v(0.2em)
    line(length: 100%, stroke: 0.6pt)
    text(1.4em, weight: "regular", title)
    v(2em)
  },
)
