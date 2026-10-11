// oracle: levels 4
// Justified text with tracking: a line sliced from its run at a safe-to-break point keeps the tracking its
// last glyph was shaped with, as Typst's does, so the lines break where Typst's do (reshaping each line
// dropped it, and five lines differed).
#set page(width: 300pt, height: auto, margin: 10pt)
#set par(justify: true)
#set text(size: 11pt, tracking: 0.6pt)
#lorem(250)
