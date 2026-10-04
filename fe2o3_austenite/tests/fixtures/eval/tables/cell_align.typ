// oracle: levels 4
// Cell align and inset fields: per-column align, a cell's own align and inset, a colspan with both, set against the table's.
#set page(width: 220pt, height: 200pt, margin: 20pt)
#table(columns: 3, align: (left, center, right), inset: 8pt,
  [a], [b], [c],
  table.cell(align: right)[right], table.cell(inset: 2pt)[tight], table.cell(align: bottom + center)[bottom#linebreak()two],
  table.cell(colspan: 2, align: center + horizon, inset: (x: 12pt, y: 3pt))[span], [z])
