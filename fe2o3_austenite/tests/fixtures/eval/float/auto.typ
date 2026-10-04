// oracle: levels 2 4
// Floats with `auto` placement: the midpoint rule sending one to the top and one to the foot, the
// clearance, a float too tall for what is left deferred to the next page in order, and `place.flush`.
#set page(width: 200pt, height: 300pt, margin: 20pt)
#block(width: 50pt, height: 30pt) <probe>
#place(auto, float: true)[#block(width: 60pt, height: 40pt) <probe>]
#block(width: 50pt, height: 30pt) <probe>
#block(width: 50pt, height: 100pt) <probe>
#place(auto, float: true)[#block(width: 60pt, height: 20pt) <probe>]
#place(auto, float: true, clearance: 5pt)[#block(width: 60pt, height: 200pt) <probe>]
#place(auto, float: true)[#block(width: 60pt, height: 10pt) <probe>]
#block(width: 50pt, height: 30pt) <probe>
#place.flush()
#block(width: 50pt, height: 30pt) <probe>
