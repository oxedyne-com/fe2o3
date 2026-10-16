// oracle: levels 2 4
// Stacks: top-to-bottom with spacing, fractional spacing filling the region, and pad around content.
#set page(width: 200pt, height: 300pt, margin: 10pt)
#stack(spacing: 5pt, [#block(width: 30pt, height: 20pt) <probe>], [#block(width: 40pt, height: 20pt) <probe>])
#pad(x: 15pt, top: 7pt)[#block(width: 30pt, height: 20pt) <probe>]
#stack([#block(width: 30pt, height: 20pt) <probe>], 1fr, [#block(width: 30pt, height: 20pt) <probe>])
