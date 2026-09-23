// U4 fold probes: each block sets a field twice and reports the value in force.
#[#set text(size: 2em)
#set text(size: 1.5em)
#context [#metadata(repr(text.size)) <p>]]
#[#set text(size: 10pt + 1em)
#context [#metadata(repr(text.size)) <p>]]
#[#set line(stroke: red)
#set line(stroke: 2pt)
#context [#metadata(repr(line.stroke)) <p>]]
#[#set rect(inset: 3pt)
#set rect(inset: (left: 1pt))
#context [#metadata(repr(rect.inset)) <p>]]
#[#set rect(inset: (x: 3pt))
#set rect(inset: (left: 1pt))
#context [#metadata(repr(rect.inset)) <p>]]
#[#set rect(radius: (top: 3pt))
#set rect(radius: (left: 1pt))
#context [#metadata(repr(rect.radius)) <p>]]
#[#set rect(radius: 2pt)
#set rect(radius: (top-left: 1pt))
#context [#metadata(repr(rect.radius)) <p>]]
#[#set rect(stroke: (left: red))
#set rect(stroke: 2pt)
#context [#metadata(repr(rect.stroke)) <p>]]
#[#set rect(stroke: red)
#set rect(stroke: (left: 2pt))
#context [#metadata(repr(rect.stroke)) <p>]]
#[#set rect(stroke: (left: red, rest: blue))
#set rect(stroke: (paint: green, thickness: 3pt))
#context [#metadata(repr(rect.stroke)) <p>]]
#[#set block(stroke: (left: red))
#context [#metadata(repr(block.stroke)) <p>]]
#[#set par(first-line-indent: 1em)
#set par(first-line-indent: (all: true))
#context [#metadata(repr(par.first-line-indent)) <p>]]
#[#set par(first-line-indent: (amount: 1em, all: true))
#set par(first-line-indent: 2em)
#context [#metadata(repr(par.first-line-indent)) <p>]]
#[#set rect(inset: (rest: 3pt))
#set rect(inset: (left: 3pt))
#context [#metadata(repr(rect.inset)) <p>]]
#[#set line(stroke: (paint: red, dash: "dashed"))
#set line(stroke: (dash: none))
#context [#metadata(repr(line.stroke)) <p>]]
#[#set line(stroke: red)
#set line(stroke: (dash: "dashed"))
#context [#metadata(repr(line.stroke)) <p>]]
#[#set page(margin: 1cm)
#set page(margin: (left: 2cm))
#context [#metadata(repr(page.margin)) <p>]]
#[#set rect(fill: red)
#set rect(fill: none)
#context [#metadata(repr(rect.fill)) <p>]]
#[#set text(features: (smcp: 1))
#set text(features: (onum: 1))
#context [#metadata(repr(text.features)) <p>]]
#[#set line(stroke: (dash: "dotted", cap: "round"))
#set line(stroke: (join: "bevel", miter-limit: 2.5))
#context [#metadata(repr(line.stroke)) <p>]]
