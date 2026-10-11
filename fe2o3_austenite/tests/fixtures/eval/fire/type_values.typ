// `stroke`, `tiling` and `selector` are types, as every other constructor is: `str` names them, `type`
// compares against them, and calling them still constructs (codly's argument checks, elembic's native types).
#let tys = (stroke, tiling, selector, color, int, function, content)
#metadata(tys.map(str)) <probe>
#metadata((type(stroke(red)) == stroke, type(selector(heading)) == selector, type(stroke) == type)) <probe>
#metadata(repr(stroke(paint: blue, thickness: 2pt).thickness)) <probe>
#show selector.or(heading, figure): set text(fill: red)
= Heading
#tys.map(str).join(", ")
