// oracle: levels 4
// The furniture's located elements come in frame order: the header's counter step falls before the body of
// its own page, so the body reads the count with its own page's step in.
#set page(width: 200pt, height: 120pt, header: [#counter("h").step()])
#context [h = #counter("h").get().first()]
#pagebreak()
#context [h = #counter("h").get().first()]
#pagebreak()
#context [h = #counter("h").get().first()]
