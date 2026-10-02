// oracle: levels 4
// A marker or cell laid out again in a region exactly as wide as it measured keeps its line whole, even
// when it ends in punctuation that overhangs (an aligned `Step 1:` in an auto column).
#let n = 1
Step #n: one

#block[Step #n: two]

#align(right)[Step #n: three]

#grid(columns: (auto, 1fr), align(right)[Step #n:], [four])
