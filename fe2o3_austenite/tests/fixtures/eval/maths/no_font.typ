// oracle: rejects
// An equation whose font list is answered by no face, with the fallback off, fails: Typst's `get_font` finds
// none. The same list with the fallback on is the control (`font_fallback.typ`).
#show math.equation: set text(font: "Zzzz Nowhere", fallback: false)
$ x + y $
