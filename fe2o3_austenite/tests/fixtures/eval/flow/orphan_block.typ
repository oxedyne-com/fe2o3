// oracle: levels 2 4
// A breakable block whose first fragment would be empty, since its body cannot start in what is left of the
// page, goes to the next page whole: its label and its fill are not left behind as an empty orphan. A second
// block, whose body is empty, stays where it is.
#set page(width: 200pt, height: 100pt, margin: 20pt)
#block(width: 100%, height: 30pt) <probe>
#block(inset: 4pt, fill: luma(235))[#block(width: 10pt, height: 40pt, breakable: false) <probe>] <probe>
#block(width: 10pt, height: 10pt) <probe>
