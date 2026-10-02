// oracle: levels 4
// A block of full width flowed paragraph by paragraph, with a note in the second, across a page break.
#set page(width: 200pt, height: 150pt, margin: 20pt)
#lorem(20)
#block(width: 100%, inset: 4pt)[First paragraph of the block.

Second paragraph with a note#footnote[In the second paragraph.] and more text to wrap onto lines.

Third paragraph, long enough to carry the block onto a second page: #lorem(30)]
