// oracle: levels 4
// Depth, indent, a custom title, no title, a long entry that wraps, and page numbers across pages.
#set page(width: 300pt, height: 300pt, margin: 25pt)
#set heading(numbering: "1.")
#outline(depth: 2, indent: 1em)
#outline(title: [Contents])
#outline(title: none)
= A heading with a title long enough to wrap onto a second line of the outline entry
== Second
=== Third (beyond the depth)
#pagebreak()
= On page two
#pagebreak()
= Page three
