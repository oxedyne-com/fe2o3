// oracle: levels 2 4
// Containers: fixed and relative block sizes, inset by side, content aligned by nothing in particular,
// clipping, a block with no body, and blocks inside a stack inside a pad.
#set page(width: 300pt, height: 400pt, margin: 25pt)
#block(width: 50%, height: 20%, inset: (left: 10pt, top: 5pt))[#block(width: 20pt, height: 20pt) <probe>] <probe>
#block(width: 100pt, height: 30pt, clip: true)[#block(width: 200pt, height: 60pt) <probe>] <probe>
#block() <probe>
#block(width: 40pt, height: 40pt, outset: 5pt) <probe>
#block(inset: 8pt)[#block(width: 30pt, height: 30pt) <probe>] <probe>
#pad(left: 20pt, rest: 5pt)[#stack(spacing: 3pt, [#block(width: 20pt, height: 20pt) <probe>], [#block(width: 20pt, height: 20pt) <probe>])]
