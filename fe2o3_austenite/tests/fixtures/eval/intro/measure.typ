// measure() lays content out in the context's styles and reports its size.
// intro: needs layout
#context [#metadata(measure(rect(width: 10pt, height: 20pt))) <probe>]
#context [#metadata(measure(block(width: 50%, height: 1em), width: 200pt)) <probe>]
#context [#metadata(measure(box(width: 3pt, height: 4pt))) <probe>]
#set text(size: 20pt)
#context [#metadata(measure(block(height: 1em))) <probe>]
