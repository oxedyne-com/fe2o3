// oracle: levels 1 4
#set page(width: 200pt, height: 160pt, margin: 15pt)
#set text(size: 9pt, lang: "fr", region: "CA", weight: "bold")
#set par(leading: 0.8em, spacing: 1.2em, justify: true, first-line-indent: 1em)
#set heading(numbering: "1.1")
#context [#metadata((
  size: text.size, lang: text.lang, region: text.region, weight: text.weight, style: text.style,
  hyphenate: text.hyphenate, tracking: text.tracking, fill: text.fill,
  leading: par.leading, spacing: par.spacing, justify: par.justify, indent: par.first-line-indent,
  width: page.width, height: page.height, margin: page.margin, numbering: page.numbering,
  numbering-heading: heading.numbering, offset: heading.offset, outlined: heading.outlined,
  marker: list.marker, figure-numbering: figure.numbering, above: block.above, below: block.below,
)) <probe>]
#context [#set text(size: 14pt); #set par(leading: 2em); #metadata((size: text.size, leading: par.leading, em: text.size * 2)) <probe>]
#context [#set page(width: 100pt, margin: (x: 5pt, y: 7pt)); #metadata((width: page.width, margin: page.margin)) <probe>]
#show heading: it => context [#metadata((level: it.level, size: text.size, weight: text.weight)) <probe> #it.body]
= A heading
#context [#metadata(text.font) <probe>]
#set text(font: ("Libertinus Serif", "DejaVu Sans Mono"))
#context [#metadata(text.font) <probe>]
#context { let s = text.size; let l = par.leading; [#metadata((s.pt(), l.em)) <probe> #(s * 3)] }
#context [#set text(weight: 500); #metadata((a: text.weight)) <probe>]
#context [#set text(weight: 650); #metadata((a: text.weight)) <probe>]
#context [#set text(weight: 700); #metadata((a: text.weight)) <probe>]
#context [#set text(font: "DejaVu Sans Mono"); #metadata(text.font) <probe>]
#context [#set text(font: ("Libertinus Serif",)); #metadata(text.font) <probe>]
