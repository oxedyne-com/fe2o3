// oracle: levels 4
// A float with scope parent in two columns, at the top and at the bottom, with clearance and dy.
#set page(width: 260pt, height: 200pt, margin: 20pt, columns: 2)
#lorem(30)
#place(top + center, float: true, scope: "parent", clearance: 6pt)[#rect(width: 120pt, height: 24pt)[Wide float]]
#lorem(50)
#place(bottom + center, float: true, scope: "parent", dy: -4pt)[#rect(width: 80pt, height: 16pt)[Bottom wide]]
#lorem(40)
