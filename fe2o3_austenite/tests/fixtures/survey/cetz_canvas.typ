#import "@preview/cetz:0.3.4": canvas, draw
#import "@preview/cetz-plot:0.1.1": plot, chart

#set page(width: 14cm, height: 14cm, margin: 5mm)

#canvas({
	import draw: *
	set-style(stroke: 0.6pt)
	circle((0, 0), radius: 1, name: "c")
	rect((2, -1), (4, 1), name: "r", fill: luma(230))
	line("c.east", "r.west", mark: (end: ">"))
	content("c", [$a$])
	content("r", [box])
	bezier((0, 1), (4, 1), (1, 3), (3, 3), stroke: blue)
	arc((5, 0), start: 0deg, stop: 270deg, radius: 0.8, stroke: red)
	grid((-1, -2), (5, -1), step: 0.5, stroke: gray + 0.3pt)
	for i in range(5) {
		line((i, -2.5), (i + 0.5, -2.2), stroke: (paint: rgb(i * 40, 80, 200), thickness: 1pt))
	}
})

#canvas({
	import draw: *
	plot.plot(size: (6, 4), x-label: [x], y-label: [y], axis-style: "school-book", {
		plot.add(x => calc.sin(x), domain: (0, 2 * calc.pi), label: [$sin$])
		plot.add(x => x * x / 10, domain: (0, 3))
	})
})
