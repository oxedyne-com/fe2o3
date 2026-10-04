#import "/src/util.typ": twice
#import "util.typ" as u
#let f(x) = twice(x) + 1
#let g = u.twice
#let part = include "../parts/part.typ"
#let data() = read("/data/d.txt")
#let rel() = read("../data/d.txt")
