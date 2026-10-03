// oracle: levels 4
// A grid whose align is a function of the cell, with a cell that sets its own align and one that sets its own inset.
#set page(width: 220pt, height: 200pt, margin: 20pt)
#grid(columns: (1fr, 1fr), inset: 4pt, align: (x, y) => if x == 0 { right } else { left },
  grid.cell(align: center)[centred], [plain], [one], grid.cell(inset: 10pt)[padded])
