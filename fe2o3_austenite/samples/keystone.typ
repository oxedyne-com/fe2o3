= The Long Life of Paper

Paper is so ordinary that we rarely stop to consider it, and yet it is one of
the *most consequential inventions* in the whole history of communication. It is
cheap, light, foldable, and extraordinarily durable when it is made well; a
sheet manufactured a thousand years ago can still be read today, while the
magnetic and optical media of the last century have already grown unreadable.
The story of how a mat of macerated plant fibre came to carry almost everything
humanity chose to remember is a story of _patient craftsmanship_, of secrets kept
and secrets stolen, and of a technology so successful that it became invisible.

The partial sums of the first natural numbers close into a tidy form:

$ sum_(i=1)^n i = frac(n (n+1), 2) $

which the flow below decides before it ends:

#figure(
	align(center, [#diagram(
		spacing: 1.5em, node-stroke: 1pt,
		node((0,2), [Start], fill: colours.light, shape: shapes.hexagon),
		edge("-|>"),
		node((0,4), align(center)[Decide?], fill: blue.lighten(50%), shape: diamond, width: 6em, height: 3.25em),
		edge("r,r,u,u,l,l", "-|>", [N]),
		edge("-|>", [Y]),
		node((0,6), [End]),
	)]),
	caption: [A small decision flow.],
)
