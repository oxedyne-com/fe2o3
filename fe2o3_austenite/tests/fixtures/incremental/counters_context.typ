// Counters and `context` reads: a page counter and its final value in the footer, a custom counter stepped through the
// text, a state updated and read back, and a read of the page the paragraph itself lands on.
#set page(width: 220pt, height: 130pt, margin: 16pt,
	footer: context [Page #counter(page).display() of #counter(page).final().first()])
#set text(size: 9pt)
#let note = counter("note")
#let seen = state("seen", 0)

First paragraph. #note.step() #seen.update(n => n + 1) It opens the account and steps the counter once, so that the later reads have something to count, and it goes on long enough to fill some lines.

#context [Note counter here: #note.get().first(), final #note.final().first(), seen #seen.get(), seen at the end #seen.final().]

Second paragraph. #note.step() #seen.update(n => n + 1) It steps both again and carries on with the sort of ordinary prose that fills a page, so that the counters move across a page break in the middle of it.

Third paragraph. #note.step() #seen.update(n => n + 1) A third step, then more text to take the document on towards its second and third pages and keep the flow going.

#context [This paragraph is on page #here().page() of #counter(page).final().first(), counter #note.get().first().]

Fourth paragraph. #note.step() #seen.update(n => n + 1) A fourth step with the text that goes with it, more or less the same as before so that the pages fill at a steady rate.

Fifth paragraph. #note.step() #seen.update(n => n + 1) The fifth step, and then the closing read of everything that has been counted.

#context [Closing read: note #note.get().first() of #note.final().first(), seen #seen.final().]
