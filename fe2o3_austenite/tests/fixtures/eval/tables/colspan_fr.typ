// oracle: levels 4
// A cell that spans a fractional column asks nothing of the auto columns it also spans: a heading across an auto column and a
// fractional one leaves the auto column at its own cells' width. Across an auto column and a fixed one it still widens the auto.
#set page(width: 300pt, height: 420pt, margin: 20pt)
#table(columns: (auto, 1fr, 2fr), table.cell(colspan: 3)[*A long group heading*], [T1], [two], [three three three], [T22], [b], [c])
#v(8pt)
#table(columns: (auto, 1fr), table.cell(colspan: 2)[*A long group heading text*], [a], [b])
#v(8pt)
#table(columns: (auto, 40pt), table.cell(colspan: 2)[*A long group heading text*], [a], [b])
#v(8pt)
#table(columns: (auto, auto, 1fr), table.cell(colspan: 2)[*A long group heading*], [x], [a], [b], [c])
