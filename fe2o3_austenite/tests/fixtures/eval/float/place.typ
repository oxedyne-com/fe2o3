// oracle: levels 2 4
// Absolute placement in the flow: the start of the flow by default, offset by dx and dy, relative offsets,
// and placement taking no room from the flow.
#set page(width: 200pt, height: 200pt, margin: 20pt)
#block(width: 30pt, height: 30pt) <probe>
#place(dx: 10pt, dy: 5pt)[#block(width: 20pt, height: 20pt) <probe>]
#block(width: 30pt, height: 30pt) <probe>
#place(dx: 50%, dy: 10%)[#block(width: 20pt, height: 20pt) <probe>]
#place(auto, float: true, dx: 7pt)[#block(width: 20pt, height: 20pt) <probe>]
#block(width: 30pt, height: 30pt) <probe>
