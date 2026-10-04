// oracle: levels 1 4
#set page(width: 200pt, height: 120pt, margin: 15pt)
#context [#metadata((
  grid-inset: grid.inset, grid-stroke: grid.stroke, grid-cell-stroke: grid.cell.stroke,
  grid-hline-pos: grid.hline.position, grid-vline-pos: grid.vline.position,
  table-inset: table.inset, table-cell-stroke: table.cell.stroke,
  table-hline-pos: table.hline.position, table-vline-pos: table.vline.position,
)) <probe>]
#context [#set table(inset: 3pt); #metadata((inset: table.inset, grid: grid.inset)) <probe>]
#context [#set grid(stroke: red); #metadata(grid.stroke) <probe>]
#context [#metadata((table-stroke: table.stroke, hline: table.hline.stroke, vline: grid.vline.stroke)) <probe>]
#context [#metadata((paint: table.stroke.paint, thickness: table.stroke.thickness, line: grid.hline.stroke.paint)) <probe>]
