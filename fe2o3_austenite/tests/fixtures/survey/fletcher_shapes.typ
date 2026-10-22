#import "@preview/fletcher:0.5.7" as fletcher: diagram, node, edge, cetz
#import fletcher.shapes: house, chevron, parallelogram, trapezium, triangle

#set page(width: 16cm, height: 18cm, margin: 6mm)
#set text(size: 9pt)

#figure(
	diagram(
		spacing: (12mm, 10mm),
		node-stroke: 0.5pt,
		debug: false,
		node((0, 0), [A], name: <a>),
		node((1, 0), [B], name: <b>, shape: house),
		node((2, 0), [C], name: <c>, shape: chevron),
		node((0, 1), [D], name: <d>, shape: parallelogram),
		node((1, 1), [E], name: <e>, shape: trapezium),
		node((2, 1), [F], name: <f>, shape: triangle),
		edge(<a>, <b>, "->"),
		edge(<b>, <c>, "->>"),
		edge(<a>, <d>, "<-"),
		edge(<d>, <e>, "|-|"),
		edge(<e>, <f>, "-/-"),
		edge(<c>, <f>, "=>", label: [#text(red)[sep]], label-sep: 1pt, label-fill: white),
		edge(<a>, "r", <e>, "->", stroke: (paint: blue, dash: "dotted")),
		edge(<b>, <e>, "->", bend: -30deg, label: $x^2$),
		edge-corner-radius: 3pt,
	),
	caption: [Several shapes.],
)

#diagram(
	cell-size: (10mm, 8mm),
	{
		let (a, b, c) = ((0, 0), (1, 0), (2, 0))
		node(a, $F(X)$)
		node(b, $G(X)$)
		node(c, $H(X)$)
		edge(a, b, "->", $alpha$)
		edge(b, c, "->", $beta$)
		edge(a, c, "->", $beta alpha$, bend: 40deg)
		for i in range(3) {
			edge((i, 0), (i, 1), "-->", label: $phi_#i$)
		}
		node((1, 2), "text node", fill: yellow.lighten(60%))
	},
)

#let d = diagram(node((0, 0), [x]), edge((0, 0), (1, 0), "->"), node((1, 0), [y]))
#box(d) and #d.
