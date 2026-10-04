// oracle: levels 4
// A background behind and a foreground over the page, each centred over the whole page, with the page
// number between them in the footer.
#set page(
	width: 200pt, height: 200pt, numbering: "1",
	background: text(30pt)[Draft],
	foreground: place(top + right, dx: -10pt, dy: 10pt)[Seal],
)
Body.
#pagebreak()
Body again.
