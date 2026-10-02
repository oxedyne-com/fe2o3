// oracle: levels 4
// `none` clears a slot together with the number it would have held; an explicit footer replaces the number.
#set page(width: 200pt, height: 200pt, numbering: "1", header: [Running head], footer: none)
Header only.
#pagebreak()
#set page(numbering: "1", header: auto, footer: [Own foot])
Own footer, no number.
#pagebreak()
#set page(numbering: "1", header: none, footer: auto)
Number again.
