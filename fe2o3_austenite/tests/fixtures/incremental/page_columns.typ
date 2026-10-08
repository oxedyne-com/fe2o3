// Page columns: two columns with a column break, a footnote inside the columns, a change to one column and then to three,
// each of which starts a page.
#set page(width: 260pt, height: 130pt, margin: 14pt, columns: 2, numbering: "1")
#set text(size: 8pt)

The first column holds the opening of the text, and it runs down the page until the column is full, when the flow moves to the top of the second column.#footnote[A note in the columns.] The text goes on in the second column with more words, which keep going until the page is full, and then they begin again on the next page.

A second paragraph is set in the same two columns, and it breaks across the foot of the first column into the head of the second where the reader meets it again.

#colbreak()

After the break the text starts at the top of the next column whatever room was left above, and it fills that column in the same way as the first one did.

#set page(columns: 1)

One column now, on a new page, with a paragraph of ordinary text that spans the full width of the page between its margins and breaks onto a second page if it must.

Another paragraph in the one column follows it, and a few more words keep the page full enough to matter.

#set page(columns: 3)

Three columns on the last pages, narrow and quick to fill, with the closing paragraphs of the document set in them one after another until the text ends.

The last paragraph is this one.
