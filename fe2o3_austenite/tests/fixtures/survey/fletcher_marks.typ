#import "@preview/fletcher:0.5.7" as fletcher: diagram, node, edge
#import fletcher.shapes: diamond, hexagon, pill, rect

#set page(width: 14cm, height: 12cm, margin: 5mm)
#set text(size: 9pt)

= Marks and shapes

#diagram(
	node-stroke: 0.6pt,
	node-fill: luma(240),
	spacing: (18mm, 12mm),
	node((0, 0), [Start], shape: pill),
	node((1, 0), [Test], shape: diamond),
	node((2, 0), [Hex], shape: hexagon),
	node((1, 1), [Box], shape: rect, name: <box>),
	node((2, 1), [Circle], shape: circle, name: <circ>),
	edge((0, 0), (1, 0), "|->"),
	edge((1, 0), (2, 0), "-|>", label: "yes"),
	edge((1, 0), <box>, "hook->", label: "no", label-side: left),
	edge(<box>, <circ>, "o-x", bend: 25deg),
	edge(<circ>, (2, 0), "~>"),
	edge((0, 0), <box>, marks: (">>", "<<"), corner: right),
	edge((2, 0), (2, 1), "..>", shift: 3pt),
	edge((2, 0), (2, 1), "<=", shift: -3pt),
)

== Marks in an array and a stroke

#diagram(
	edge-stroke: 1pt + blue,
	node((0, 0), $X$),
	node((1, 0), $Y$),
	node((0, 1), $Z$),
	edge((0, 0), (1, 0), "-->"),
	edge((0, 0), (0, 1), "=>"),
	edge((1, 0), (0, 1), "<==>", crossing: true),
	edge((0, 0), (0, 1), marks: ("|", ">"), stroke: red, label: [m]),
)

#diagram(
	spacing: 1.2cm,
	node((0, 0), [one], inset: 4pt, stroke: 1pt, corner-radius: 3pt),
	node((1, 0), [two], inset: 4pt, stroke: 1pt, corner-radius: 3pt),
	node((2, 0), [three], inset: 4pt, stroke: 1pt, corner-radius: 3pt),
	node(enclose: ((0, 0), (1, 0)), stroke: (dash: "dashed"), inset: 8pt, [group]),
	edge((0, 0), (1, 0), "->", label-pos: 0.3, label: [x]),
	edge((1, 0), (2, 0), sym.arrow.r.long),
	edge((2, 0), (2, 0), "->", bend: 130deg, loop-angle: 90deg),
)
