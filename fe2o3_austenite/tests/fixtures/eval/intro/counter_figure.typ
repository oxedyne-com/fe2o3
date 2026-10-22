// Figures count per kind; an unnumbered figure does not count.
// intro: needs figure numbering
#figure(rect(), caption: [one])
#figure(table[a], caption: [t])
#figure(rect(), caption: [two]) <two>
#figure(rect(), numbering: none)
#context [#metadata((counter(figure.where(kind: image)).get(), counter(figure.where(kind: table)).get(), counter(figure.where(kind: image)).at(<two>), query(figure).len())) <probe>]
