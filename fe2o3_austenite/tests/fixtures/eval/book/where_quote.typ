// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show quote.where(block: true): it => [BQ #it.body]
#show quote.where(block: false): it => [IQ #it.body]
#quote(block: true)[b]
#quote[i]
#quote(block: false)[j]
