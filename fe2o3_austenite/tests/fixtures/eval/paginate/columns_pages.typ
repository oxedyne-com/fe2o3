// `columns(2)[..]` across pages: the body flowed through the two columns of each page, then the text after.
#set page(width: 260pt, height: 200pt, margin: 20pt)
#set text(size: 10pt)
#lorem(12)
#columns(2, gutter: 12pt)[#lorem(260)]
#lorem(20)
