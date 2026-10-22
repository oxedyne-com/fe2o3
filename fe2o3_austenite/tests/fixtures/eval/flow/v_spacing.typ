// oracle: levels 2 4
// Vertical spacing: strong spacing, weak spacing collapsing into block spacing and dropped at the top of
// a page, and relative spacing against the page's height.
#set page(width: 200pt, height: 300pt, margin: 10pt)
#v(5pt, weak: true)
#block(width: 40pt, height: 10pt) <probe>
#v(20pt)
#block(width: 40pt, height: 10pt) <probe>
#v(5pt, weak: true)
#block(width: 40pt, height: 10pt) <probe>
#v(40pt, weak: true)
#block(width: 40pt, height: 10pt) <probe>
#v(10%)
#block(width: 40pt, height: 10pt) <probe>
#v(-5pt)
#block(width: 40pt, height: 10pt) <probe>
#pagebreak()
#v(30pt)
#block(width: 40pt, height: 10pt) <probe>
