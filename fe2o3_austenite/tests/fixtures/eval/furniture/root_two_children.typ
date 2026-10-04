// oracle: levels 4
// Two paragraphs, each sized by its own call: nothing is shared between them, so the footer keeps the
// page's size as well.
#set page(width: 200pt, height: 160pt, margin: (x: 20pt, y: 30pt), numbering: "1", header: [Head])
#text(size: 16pt)[First sized.]
#text(size: 9pt)[Second sized.]
