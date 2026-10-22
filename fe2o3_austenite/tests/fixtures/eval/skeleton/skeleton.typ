// oracle: levels 1 3 4
// The walking skeleton (streaming addendum, section 5). It holds no <probe>, so level 1 is not applicable;
// level 2 differs: Typst places the heading's tag at the baseline of its line, 12.64pt below the line's top.
#set text(size: 14pt)
#show heading: it => [Section: #it.body]
= Skeleton
A plain paragraph long enough to wrap onto a second line at the default measure, so the line breaker runs.
