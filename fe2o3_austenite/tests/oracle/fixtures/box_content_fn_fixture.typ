// The styled-box content-fn regression root (the silent content-loss SEV). A content function whose body is
// wrapped in a STYLED BOX -- `#let stamp(s) = box(fill: .., outset: .., radius: ..)[*v: #s*]` -- dropped its
// inner text SILENTLY: the definition was captured by neither the furniture reader (which drops a content
// group trailing the wrap's `)`) nor the content reader (which required the body to open with `[`), so a
// `#stamp("v2")` call rendered an EMPTY GAP where "v: v2" should be, with a clean compile. The fix captures
// the wrap's INNER `[...]` content as the content binding's body and records the box's own styling -- which
// this reader does not draw -- as a VISIBLE skip, so the text is always set and the styling loss is never
// silent.
//
// Deliberately self-contained -- a local `#let doc`, local `#let stamp` / `#let tag` / `#let badge`, bodies
// wrapped only in `*strong*` -- so it needs no symbol-modifier patch to compile under typst 0.15.1, unlike
// the two `oxeweb` roots. The box/rect/block wrappers carry fill/outset/radius/stroke/inset styling: typst
// draws the styled box, austenite sets the text plain and records the wrapper as a skip. The RENDERED TEXT
// is what this root asserts is byte-faithful; the box styling is where the two engines part, by design (see
// the crate's `content_bindings.rs`, which proves the text is present and the styling flagged at unit level).
//
// Non-vacuous: reverting the fix leaves `#stamp(...)` captured by neither reader, so its text vanishes and a
// silent gap returns -- the set text and the baseline both move.
#let doc(body, heading-font: "Libertinus Serif") = body
#show: doc.with(heading-font: "Libertinus Serif")

#let stamp(s) = box(fill: luma(240), outset: 2pt, radius: 3pt)[*v: #s*]
#let tag(s) = rect(stroke: 1pt)[*t: #s*]
#let badge(s) = block(inset: 6pt)[*b: #s*]

= The Styled Box Field Guide

Consult the build marked #stamp("v2") in the running prose, and the tag #tag("beta") beside it, so each
inline styled-box call sets its own argument between the words before and after it. A second call must read
its own argument, so #stamp("v3") and #stamp("v2") must set two different values, not one repeated fallback.

The own-line forms stand as their own blocks:

#stamp("v2")

#tag("beta")

#badge("ready")

This closing paragraph follows the own-line calls, so a regression that dropped a styled-box call or the
prose around it would move both the set text and the baseline. The heading stands at level one, so it carries
no section number under either engine.
