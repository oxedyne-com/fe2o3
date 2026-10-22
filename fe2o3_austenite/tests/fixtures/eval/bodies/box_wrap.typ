// oracle: levels 4
// A box with a relative width inside a line, and a fractional one beside text; its body wraps inside it.
#set page(width: 300pt, height: 300pt, margin: 25pt)
Start #box(width: 50%, stroke: 0.5pt, inset: 2pt)[a body that is long enough to wrap onto a further line inside the box] end.

Fill #box(width: 1fr, stroke: 0.5pt)[left] done.
