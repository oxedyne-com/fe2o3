// query: ref footnote quote link raw
#set heading(numbering: "1.")
= First <first>
#figure(rect(), caption: [Box]) <box>
See @first and @box, @first[Chapter] and #ref(<box>, supplement: none).
Note#footnote[A note.] <fn> and again#footnote(<fn>).
#quote(attribution: [Someone])[Words]
#quote(block: true)[Block]
#link("https://typst.app")[Typst] and https://example.org and #link(<first>)[back].
Inline `raw` and
```rust
fn main() {}
```
