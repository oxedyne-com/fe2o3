Hello, world! It's "quoted" -- and --- dashes... ~ nbsp -? soft -1 minus
// ---- case ----
*strong* _emph_ *nested _both_ here* and *multi word strong*
// ---- case ----
a*b*c and a_b_c and snake_case_name and 中*文*字 and 한_글_
// ---- case ----
= One
== Two <l2>
=Not heading
 x = y
=== Three *bold* _it_
// ---- case ----
- a
  - b
  - c
- d
 - e
text after
// ---- case ----
+ a
+ b
7. seven
 12. twelve
1.5 not an item
2024. was a year
// ---- case ----
/ Term: desc
/ Other: more
  continued
// ---- case ----
`inline` `` and ``two``
```rust
  fn main() {}
    indented
  ```
````
four ``` inside
````
``` trimmed ```
```
  first
last```
// ---- case ----
Links https://typst.app/docs?x=1&y=(2). and http://a.b/c:d, then (https://x.y/z) end.
http and https: are words
// ---- case ----
See @fig:one and @sec.x. and @a[Supplement *b*] text <lbl> and @b_c: again <two>
// ---- case ----
\# \* \_ \u{1F600} \\ \$ \[ \] \< \@ \~ \-
// ---- case ----
a \
b\
c

d


e
// ---- case ----
[a [b] c] and [nested [deep [er]]]
// ---- case ----
#[*in* block] and #"str" and #12 and #none and #[] end
// ---- case ----
a: b / c x / y
- list: item
// ---- case ----
= Title // comment
text /* block /* nested */ */ more // tail
// ---- case ----
- one

- two
  continuing

  new para in item
- three
// ---- case ----
Ünïcödé text with émojis 😀 and CJK 中文字 and RTL שלום.
// ---- case ----
$x$ and $ x^2 $ and $$ and $x $ end
// ---- case ----
It's John's "book" 'single' and don't
// ---- case ----
#!/usr/bin/env typst
first line
// ---- case ----
_emph *strong inside* emph_ *strong _emph inside_ strong*
// ---- case ----
- *bold item*
  + nested enum
    / t: d
- `code` item
// ---- case ----
1. first
2. second

3. third
// ---- case ----
text with trailing space   
and tab	indent
// ---- case ----
==
= 
- 
+ 
