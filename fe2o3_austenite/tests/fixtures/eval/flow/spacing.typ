// oracle: levels 2 4
// Block spacing: the default 1.2em, explicit above and below, `set block(spacing:)`, and the larger of two
// meeting spacings winning.
#set page(width: 200pt, height: 400pt, margin: 10pt)
#block(width: 50pt, height: 20pt) <probe>
#block(width: 50pt, height: 20pt) <probe>
#block(width: 50pt, height: 20pt, above: 30pt) <probe>
#block(width: 50pt, height: 20pt, below: 4pt) <probe>
#block(width: 50pt, height: 20pt, above: 2pt) <probe>
#set block(spacing: 7pt)
#block(width: 50pt, height: 20pt) <probe>
#block(width: 50pt, height: 20pt) <probe>
