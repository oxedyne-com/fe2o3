// oracle: levels 4
// place with dx and dy offsets, in the flow and as a float.
#set page(width: 220pt, height: 160pt, margin: 20pt)
Text before.
#place(right, dx: -10pt, dy: 12pt)[#rect(width: 30pt, height: 14pt)[a]]
#place(center, dy: 40pt)[#rect(width: 30pt, height: 14pt)[b]]
#place(top + left, dx: 6pt, dy: 6pt, float: true)[#rect(width: 30pt, height: 14pt)[c]]
Text after the places that wraps over a couple of lines to be sure of the flow.
