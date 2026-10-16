// oracle: levels 2 4
// needs: U6a
// Spacing in em follows the font size in force: the default paragraph spacing and `set block(spacing:)`.
#set page(width: 200pt, height: 300pt, margin: 10pt)
#set text(size: 20pt)
#block(width: 50pt, height: 20pt) <probe>
#block(width: 50pt, height: 20pt) <probe>
#set block(spacing: 1em)
#block(width: 50pt, height: 20pt) <probe>
#block(width: 50pt, height: 20pt) <probe>
