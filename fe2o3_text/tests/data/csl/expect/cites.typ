// Citations for tests/csl_render.rs: compile with `--input style=<name>` to `cites-<name>.html`.
// Each list item is one citation; the test builds the same list.
#let s = sys.inputs.at("style", default: "ieee")
+ @a1
+ @n1
+ @n1 @n2 @n3
+ @n5 @n4
+ @n1[p. 5]
+ @n1[p. 5]
+ @n1
+ @n2 @n3 @a1
+ #cite(<n1>, form: "author")
+ #cite(<n1>, form: "year")
+ #cite(<n1>, form: "prose")
+ #cite(<n1>, form: "full")
#bibliography("refs.bib", style: s, title: none)
