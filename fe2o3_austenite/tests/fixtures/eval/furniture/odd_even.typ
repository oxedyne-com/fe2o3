// oracle: levels 4
// A two-sided page: inside and outside margins, and a header that differs on odd and even pages by the
// page counter, on the side of the page its parity gives.
#set page(
	width: 200pt, height: 200pt, margin: (inside: 36pt, outside: 18pt, y: 40pt),
	header: context {
		let p = counter(page).get().first()
		if calc.odd(p) { align(right)[Odd #p] } else { align(left)[Even #p] }
	},
)
Page one text.
#pagebreak()
Page two text.
#pagebreak()
Page three text.
#pagebreak()
Page four text.
