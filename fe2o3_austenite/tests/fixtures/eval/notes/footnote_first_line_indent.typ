// oracle: levels 4
// A footnote entry is a paragraph's worth of inline content, so a first-line indent set for every paragraph
// does not reach it: the entry keeps its own indent, whatever the amount, in or out of columns.
#set page(width: 300pt, height: 300pt, margin: 25pt)
#set text(size: 10pt)
#set par(first-line-indent: (amount: 1.5em, all: true), justify: true)
First paragraph with a note.#footnote[The body of the first note, long enough to wrap onto a second and a third line at the foot of the page.] More text follows it. Another sentence with a second note.#footnote[A second, shorter note.]

A second paragraph with a third note.#footnote[#lorem(30)]
