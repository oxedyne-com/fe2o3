// oracle: levels 2 4
// Fractional spacing: `v(1fr)` against `v(2fr)` sharing the free space, a fractional block height, and a
// fractional block among spacing.
#set page(width: 200pt, height: 300pt, margin: 10pt)
#block(width: 30pt, height: 20pt) <probe>
#v(1fr)
#block(width: 30pt, height: 20pt) <probe>
#v(2fr)
#block(width: 30pt, height: 20pt) <probe>
#pagebreak()
#block(width: 30pt, height: 20pt) <probe>
#block(width: 30pt, height: 1fr) <probe>
#block(width: 30pt, height: 20pt) <probe>
#pagebreak()
#block(width: 30pt, height: 2fr) <probe>
#v(1fr)
#block(width: 30pt, height: 20pt) <probe>
