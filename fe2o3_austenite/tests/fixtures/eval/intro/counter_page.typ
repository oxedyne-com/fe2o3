// The page counter follows the pages, its updates, and each page's numbering.
// intro: needs layout numbering furniture
#set page(width: 200pt, height: 200pt, numbering: "1")
#context [#metadata((counter(page).get(), here().page-numbering())) <probe>]
#pagebreak()
#counter(page).update(10)
#context [#metadata((counter(page).get(), counter(page).display(), counter(page).final())) <probe>]
#pagebreak()
#set page(numbering: "i")
#context [#metadata((counter(page).get(), counter(page).display(), here().page-numbering(), counter(page).display("1 / 1", both: true))) <probe>]
