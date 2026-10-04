// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show regex("[Ss]ome\\w*"): it => strong(it)
#show "fox": set text(red)
#show <lbl>: set text(blue)
#show emph: set text(green)
#show strong: it => [<#it.body>]
#show link: underline
#set smartquote(quotes: (double: ("«", "»"), single: ("‹", "›")))
Something about a fox, *bold* _slanted_ "quoted" it's [lab] <lbl> #link("https://x.org")[link]
