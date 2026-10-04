// oracle: levels 4
// A set rule on the text font with the fallback off does not reach an equation, whose own show-set names its
// face; the surrounding text takes none.
#set page(width: 200pt, height: 100pt, margin: 20pt)
#set text(font: "Zzzz Nowhere", fallback: false)
$ x + y $
