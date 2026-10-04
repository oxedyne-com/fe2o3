// oracle: levels 4
// A grid or table is as wide as its columns, so `align` places it: centred or at the right, the cells move with it. A fractional
// column fills the region, and fixed columns wider than the region leave the grid at the region's start, whatever the alignment.
// The same holds in a block of its own width and in a box.
#set page(width: 200pt, height: 330pt, margin: 10pt)
#align(center, grid(columns: (40pt, 40pt), [alpha], [beta]))
#align(right, table(columns: (40pt, 40pt), [gamma], [delta]))
#align(center, grid(columns: (1fr, 1fr), [one], [two]))
#align(right, grid(columns: (40pt, 1fr), [three], [four]))
#align(center, grid(columns: (110pt, 110pt), align: left, [wide], [wider]))
#align(right, table(columns: (100pt, 100pt), align: left, [over], [flow]))
#align(center, table(columns: 2, [auto], [columns], [a], [b]))
#block(width: 100%, align(center, grid(columns: (40pt, 40pt), column-gutter: 10pt, [in], [block])))
#box(width: 150pt, align(right, grid(columns: (30pt, 30pt), [in], [box])))
