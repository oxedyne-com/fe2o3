// A text show rule matches a symbol, which realises as text before the rules run, and its output
// stays in the paragraph.
// oracle: levels 3
#show "–": it => [(#it)]
#show "…": it => [«#it»]

Between a -- b and c... the rules apply.
