// oracle: levels 4
// A set rule is liftable: `set text` ahead of the page's only content reaches the footer and header, as
// Typst lifts a set rule's styles to the page level.
#set page(width: 200pt, height: 160pt, margin: (x: 20pt, y: 30pt), numbering: "1", header: [Head])
#set text(size: 15pt)
Only a lifted size.
