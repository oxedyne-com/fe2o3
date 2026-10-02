// query(heading): the headings in document order with their fields, and the bounds `before` and `after`
// put on them, read from contexts that see the whole document.
// intro: needs heading numbering
#set heading(numbering: "1.")
#context [#metadata(query(heading).map(h => (h.level, h.body))) <probe>]
= One
== Two <two>
= Three
#context [#metadata((query(heading.where(level: 1)).len(), query(selector(heading).before(<two>)).len(), query(selector(heading).after(<two>)).len(), locate(<two>).page())) <probe>]
