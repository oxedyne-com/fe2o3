// Headings that are referenced and outlined: the outline lists every heading with its page, the text refers forward
// and back, and a page read through a counter at a label.
#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: "1")
#set text(size: 9pt)
#set heading(numbering: "1.1")
#outline(depth: 2)

= Introduction <intro>

This part introduces the plan and points ahead to @method and to @results, which come later in the document.

= Method <method>

The method is described in steps. It is set out here at some length, since the later parts depend on it and refer back to @intro for the reasons.

== Details <details>

Details follow, with the page of the introduction read as #context counter(page).at(<intro>).first() and the page of the results as #context counter(page).at(<results>).first().

== More details <more>

More details, and a few lines of ordinary text to take the section on towards a new page so that the headings after it move.

= Results <results>

The results come last. They are described in @details and in @more, and they close the document with some further text for the final page.

== Summary <summary>

A short summary of everything above, referring to @method once more, and ending the document.
