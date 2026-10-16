// `text`'s body is the first positional argument left once a positional size and fill are found, cast to
// content: `none` is empty text, a symbol a symbol (touying, nerd-icons, zero, list markers).
#set list(marker: text(fill: blue, sym.square.filled))
- One #text(red, none)end
- Two #text(0.9em, sym.arrow.r) #text(green, 1.2em, sym.bullet) #text(size: 8pt, sym.checkmark)
#metadata(repr(text(red, none))) <probe>
#metadata(text(1em, sym.alpha).func() == text) <probe>
#context [#metadata(measure(text(1em, sym.zws)).height > 0pt) <probe>]
