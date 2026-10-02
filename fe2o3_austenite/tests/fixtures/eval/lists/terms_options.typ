// oracle: levels 4
// Separator, indent, hanging indent, a wide list and a term with a long name.
#set page(width: 300pt, height: 500pt, margin: 25pt)
#terms(separator: [ -- ], indent: 10pt, hanging-indent: 18pt,
  terms.item[Short][a description],
  terms.item[Very long term name that wraps][another description])
#set terms(tight: false)
/ One: wide first
/ Two: wide second
#set terms(tight: true, hanging-indent: 0pt, spacing: 8pt)
/ Flat: no hanging indent but a spacing of eight points set explicitly on this list
/ Next: item
