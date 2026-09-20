// This crate's own term-dict-inside-a-content-fn fixture (reader-completeness item 4). The regression it
// guards: a term-dictionary lookup nested inside a `#let name(params) = [ ... ]` content-fn body, keyed on
// the fn's OWN parameter -- `#let cite-term(w) = [Learn about #t(w).]`, called as `#cite-term("website")`
// -- must resolve against the caller's argument, not against the literal parameter name. Before the fix,
// `expand_content_body` only substituted a bare `#param` markup-mode reference; `w` inside `#t(w)` carries
// no `#` of its own (it is already Typst code once the parenthesis opens), so it passed through
// unsubstituted, `#t` read the literal text "w" as the key, found no such entry, and rendered the fallback
// "w" with a recorded "#t unknown term-dict key" skip. The fix substitutes a parameter reference in a call's
// argument list too, quoting the caller's value, so the nested `#t` sees "website" and "org" -- the two
// calls below must render two DIFFERENT resolved values, not the same fallback text, proving the
// substitution is genuine rather than coincidental.
//
// `terms.typ` beside this file is both engines' single source for the dictionary: the reader finds it by
// walking up from this file's own directory (see that file's own comment), and the `#import` below hands
// the same values to real Typst. The reader treats `#import` as an unhandled construct and skips it
// visibly -- harmless here, since `t` is answered by the reader's own hardcoded term-dictionary
// recognition, never by evaluating the import. The local `t` below is a plain dictionary read with no
// first-use styling, deliberately far simpler than elearnity's real `style/glossary_index.typ`, since real
// Typst only needs a working `t` to render this fixture -- and, written as a `{ ... }` code body rather
// than a `[ ... ]` content body or a `block`/`box` wrap, it is not itself mistaken for a content-fn or
// furniture binding by `collect_content_fns`/`collect_template_fns`.

#import "terms.typ": term-dict

#let t(key) = { term-dict.at(key) }

#set page(width: 595.276pt, height: 841.89pt, margin: 56.9pt)
#set text(size: 11pt, font: "Libertinus Serif")
#set par(justify: true, leading: 0.65em, spacing: 1.2em, first-line-indent: 0pt)

= Term Dictionary In A Content Function

Read directly, with no content function in between, the dictionary still resolves as plain prose: our
organisation is #t("org"), reachable at #t("website"). This paragraph is written long enough to wrap onto
more than one line, so the page carries real body text beyond the two reference-bearing sentences below.

#let cite-term(w) = [Learn about #t(w).]

#cite-term("website")

#cite-term("org")
