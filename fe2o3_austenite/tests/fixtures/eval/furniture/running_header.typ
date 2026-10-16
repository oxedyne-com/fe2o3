// oracle: levels 4
// A running header from a context query: the last level-one heading at or before the page. The first pass
// has no introspector, so the header settles only when its reads are compared and a second pass is made.
#set page(
	width: 220pt, height: 200pt, margin: (top: 46pt, bottom: 30pt, x: 24pt),
	header: context {
		let hs = query(heading.where(level: 1).before(here()))
		if hs.len() > 0 { emph(hs.last().body) } else { [no chapter yet] }
	},
)
#set heading(numbering: none)
Preface text.
#pagebreak()
= Alpha
Alpha text.
#pagebreak()
Alpha continues.
#pagebreak()
= Beta
Beta text.
