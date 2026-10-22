// `page.width` and `page.height` read in context are the nearer of their own setting and a paper, A4 when
// neither is set, and are not swapped by `flipped` (touying, hydra, showybox).
#context [#metadata((page.width, page.height)) <probe>]
#set page(paper: "a5", flipped: true)
#context [#metadata((page.width, page.height)) <probe>]
#set page(height: auto)
#context [#metadata((page.width, page.height)) <probe>]
#set page(width: 300pt, paper: "us-letter")
#context [#metadata((page.width, page.height)) <probe>]
Body text.
