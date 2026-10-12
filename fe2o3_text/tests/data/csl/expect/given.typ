// Given-name disambiguation for tests/csl_render.rs: compile with `--input style=<name>` to
// `given-<name>.html`. Two Smiths, one work each, so that only their given names tell them apart.
#let s = sys.inputs.at("style", default: "modern-language-association")
+ @d1
+ @d4
+ @d1 @d4
+ @d5 @d6
#bibliography("group.bib", style: s, title: none)
