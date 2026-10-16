// oracle: levels 4
// Roman front matter, then arabic from one again after a counter reset.
#set page(width: 200pt, height: 200pt, numbering: "i")
Front matter one.
#pagebreak()
Front matter two.
#set page(numbering: "1")
#counter(page).update(1)
Body one.
#pagebreak()
Body two.
#pagebreak()
Body three.
