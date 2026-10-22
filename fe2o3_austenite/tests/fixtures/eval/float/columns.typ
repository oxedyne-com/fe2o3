// oracle: levels 2 4
// Floats in a two-column page: a column float, a parent-scoped float spanning both columns and relaying
// the page out beneath it, and three floats contending for one page.
#set page(width: 300pt, height: 300pt, margin: 20pt, columns: 2)
#block(width: 40pt, height: 50pt) <probe>
#place(auto, float: true)[#block(width: 80pt, height: 30pt) <probe>]
#block(width: 40pt, height: 50pt) <probe>
#place(auto, float: true, scope: "parent")[#block(width: 200pt, height: 40pt) <probe>]
#block(width: 40pt, height: 50pt) <probe>
#block(width: 40pt, height: 50pt) <probe>
#place(auto, float: true)[#block(width: 80pt, height: 90pt) <probe>]
#place(auto, float: true)[#block(width: 80pt, height: 90pt) <probe>]
#place(auto, float: true)[#block(width: 80pt, height: 90pt) <probe>]
#block(width: 40pt, height: 50pt) <probe>
