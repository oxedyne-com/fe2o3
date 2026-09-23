// Markup as content: the evaluated tree of emphasis, strong, raw, links, breaks and labels.
#metadata([Plain *strong* _emph_ `raw` text]) <probe>
#metadata([Line one \ line two]) <probe>
#metadata([A #link("https://example.org")[link] and a label <lab>]) <probe>
#metadata([- item a
- item b]) <probe>
#metadata([+ first
+ second]) <probe>
#metadata([/ Term: definition]) <probe>
#metadata([== Sub heading]) <probe>
#metadata([Quotes "double" and 'single' -- dash --- em ... dots ~ nbsp]) <probe>
#metadata(heading(level: 2)[Built]) <probe>
#metadata(strong[a] + emph[b]) <probe>
#metadata([#h(1em)#v(2pt)#box[b]#block[c]]) <probe>
