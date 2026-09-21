# oxeweb-techspec re-pin — 2026-09-21

Golden-file re-pin of the `oxeweb-techspec` oracle root, following the inline
mid-prose content-fn expansion change (reader-completeness item 3). Lead
sign-off: coordinator, rc-3 slot.

## Hash change

| | sha256 |
|---|---|
| BEFORE (`bac7751`, current pin) | `9a142b10b67a29a48cf151e9705bb90d6911ab3dbf8f56cc89c4f705185d400d` |
| AFTER (this change)             | `18c263888e8cc159cce1b96cbf9cc9fee1617eeb56a99ed9dff1e9fc48e44708` |

Pages 141 → 141, anchors 574 → 574 (structure unchanged; only inline `#oxe`
glyph occurrences differ).

## Why this is a self-pin, not a Typst-validated pin

`oxeweb-techspec` has NO Typst oracle: `typst` 0.15.1 cannot compile it. Its
`utils.typ` (shared with `oxeweb-overview`) defines `cat()`/`tup()` helpers
using the `bracket.l.double` / `angle.l.double` symbol modifiers, **removed in
0.15.1** with no glyph-identical replacement. `typst query` on the harness
dump exits 1, so the harness has always downgraded this root's oracle
comparison to a note (see `tests/oracle/mod.rs::corpus`). The pin is therefore
austenite's own output, and this re-pin is backed by human-legible visual
evidence rather than byte-acceptance.

## What changed, and why it is a fidelity improvement

The document's `#let oxe = [#h(..)#box(.. scale(..)[$times.circle$] ..)#h(..)]`
binding is referenced inline mid-prose — `M#oxe`, `B#oxe` (ch08_tokens.typ).

- **BEFORE:** an inline reference to a content binding was not expanded, so the
  raw source `M#oxe` / `B#oxe` **leaked onto the page verbatim**, and was NOT
  recorded as a refusal (a silent, untallied raw-markup leak).
- **AFTER:** the reference expands, exactly as an own-line reference already
  does. The binding's body is Typst layout primitives austenite does not
  implement (`#h`, `#box`, `scale`), so they are refused and TALLIED in the
  skip report (`#h ×4, #box ×2`), setting no ink where the ⊗ glyph would go.
  The raw `#` no longer reaches the page.

Typst itself would set the scaled circled-times glyph (`M⊗`); austenite can do
neither engine's glyph without implementing `#box`/`scale`/`#h`, so the honest
outcome is a tallied refusal. The change converts a **silent raw-markup leak
into a tallied, clean refusal** — strictly better observability and no `#`
garbage on the page.

## Visual evidence (page 78, "Tokens / Units")

- `page78_before_raw_oxe_leak.png` — four raw leaks: list items "written as
  one moxe or **M#oxe**." / "written as **B#oxe**.", and the paragraph "One
  full oxecoin is approximately **M#oxe 4,295 or B#oxe 4.3**."
- `page78_after_refused.png` — the same four occurrences now set "M." / "B." /
  "M 4,295 or B 4.3" with the raw `#oxe` gone.

Every other pixel on the page is identical between the two renders (same
headings, same 1/2/3 list structure, same paragraphs, same line breaks, same
folio and footer): the ONLY delta is the four `#oxe` occurrences. No corruption,
no structural change.

## Reproduction

Rendered with the austenite binary at 150 DPI, page 78 of 141. BEFORE was
produced from the same tree with the inline-expansion pre-pass disabled, whose
full-document hash reproduced the current pin `9a142b10…` exactly — confirming
the probe is a faithful stand-in for `bac7751`.

All 16 other pinned roots stay byte-identical; only this root moves.
