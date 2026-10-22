#import "@preview/fletcher:0.5.7" as fletcher: diagram, node, edge

#set page(width: 12cm, height: 8cm, margin: 5mm)

A commutative square with string marks and symbol marks.

#align(center, diagram(
	node-stroke: 0.5pt,
	spacing: (24mm, 16mm),
	node((0, 0), [$A$]),
	node((1, 0), [$B$]),
	node((0, 1), [$C$], shape: circle),
	node((1, 1), [$D$]),
	edge((0, 0), (1, 0), "->", label: [$f$]),
	edge((0, 0), (0, 1), sym.arrow.r, label: [$g$], label-side: right),
	edge((1, 0), (1, 1), sym.arrow.double.long.r),
	edge((0, 1), (1, 1), "<->", label: [$h$], bend: 15deg),
	edge((0, 0), (1, 1), sym.arrow.r.l, dash: "dashed"),
))

#diagram(
	node((0, 0), [In]),
	node((1, 0), [Out]),
	edge((0, 0), (1, 0), sym.arrow.l, label: [back]),
	edge((0, 0), (1, 0), "=>"),
	edge((0, 0), (1, 0), sym.arrow.bar, bend: -20deg),
)
