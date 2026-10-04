// query: figure
#figure(rect(), caption: [A rectangle]) <rect>
#figure(table[a], caption: [A table])
#figure([plain], numbering: none)
#figure(rect(), kind: table, caption: [Forced table])
#set figure(gap: 1em)
#figure(rect(), caption: none, placement: auto)
#figure(`code`, caption: [Listing])
#figure([custom], kind: "diagram", supplement: [Diagram], caption: [Custom kind])
#set figure.caption(separator: [. ])
#figure(rect(), caption: [Separated])
