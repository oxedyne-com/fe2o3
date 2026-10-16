// Page breaks with parity: a section that must start on an odd page and one on an even page, so that a blank page is
// left or not according to how many pages the section before takes.
#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: "1",
	footer: context [Page #counter(page).display() of #counter(page).final().first()])
#set text(size: 9pt)

The first section sits on the first page and runs on for a few lines, long enough to matter but short enough to leave room.

It has a second short paragraph, so that the page is nearly full before the break that follows it.

#pagebreak(to: "odd")

The second section starts on an odd page. Whether a blank page lies before it depends on how many pages the first section took.

A second paragraph of the second section, and a little more text to carry the section on towards the end of its page.

A third paragraph, which may or may not fall onto a new page, depending on the lines above.

#pagebreak(to: "even")

The third section starts on an even page, and so a blank page may lie before it too, numbered and furnished like the rest.

Its second paragraph is here, and then the document ends.
