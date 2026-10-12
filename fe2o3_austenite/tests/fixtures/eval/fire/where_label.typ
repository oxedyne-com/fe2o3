// `where(label: ..)` filters on the label every element carries outside its schema, as Typst's field 255
// (codly 1.3.0's `raw.line.where(label: ..)`, bookly's `metadata.where(label: ..)`).
#show metadata.where(label: <title>): it => it.value
#show heading.where(label: <a>): set text(red)
#metadata("Shown")<title>
#metadata("Hidden")
= Labelled <a>
= Plain
#context query(heading.where(label: <a>)).len()
#repr(heading.where(label: <a>))
