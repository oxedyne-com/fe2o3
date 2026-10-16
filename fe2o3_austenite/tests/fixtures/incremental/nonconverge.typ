// A document that never settles: a state that feeds itself, read where nothing shows. Typst stops after five attempts
// with a warning and keeps the last pass, and so must a warm compile.
#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: "1")
#set text(size: 9pt)
#let s = state("s", 0)
#context s.update(s.final() + 1)
#context [#metadata(s.final()) <probe>]

The first paragraph is plain text with nothing in it that depends on the loop, set before the state is read and long enough to fill a few lines.

A second paragraph follows, and a third after it, so that there are several places in the text where a single letter can be changed.

The third paragraph carries on a little, with a few more words, so that the page is a good deal fuller than it was before.

The fourth paragraph is here so that the document runs onto a second page, where the warning's effect on the page count could show.

The fifth paragraph is a short one, and the sixth follows it.

The sixth paragraph ends the document with a sentence or two about nothing in particular, which is all that it is for.
