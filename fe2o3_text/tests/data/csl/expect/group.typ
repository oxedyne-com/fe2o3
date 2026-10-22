// Grouping, collapsing and disambiguation for tests/csl_render.rs: compile with
// `--input style=<name>` to `group-<name>.html`. Each list item is one citation.
#let s = sys.inputs.at("style", default: "apa")
+ @d1 @d2 @d3
+ @d1 @d4 @d3 @d2
+ @d4
+ @d5
+ @d6
+ @d5 @d6 @d1
+ @d3 @d1 @d2 @d5 @d6
+ @d7
+ @d3 @d7
#bibliography("group.bib", style: s, title: none)
