// oracle: levels 2 4
// Page breaks: strong and weak, consecutive strong breaks leaving a blank page, `to: "odd"`, a trailing
// break, and blocks too tall for what is left of a page moving to the next.
#set page(width: 150pt, height: 150pt, margin: 15pt)
#block(width: 30pt, height: 50pt) <probe>
#block(width: 30pt, height: 50pt) <probe>
#block(width: 30pt, height: 30pt) <probe>
#pagebreak(weak: true)
#pagebreak(weak: true)
#block(width: 30pt, height: 10pt) <probe>
#pagebreak()
#pagebreak()
#block(width: 30pt, height: 10pt) <probe>
#pagebreak(to: "odd")
#block(width: 30pt, height: 10pt) <probe>
#pagebreak(to: "even")
#block(width: 30pt, height: 10pt) <probe>
#pagebreak()
