# Third-party licences

`typst-LICENSE.txt` and `typst-NOTICE.txt` are the unmodified `LICENSE` and `NOTICE` files from the
[typst/typst](https://github.com/typst/typst) repository at tag `v0.15.1`, fetched
2026-09-23 from `https://raw.githubusercontent.com/typst/typst/v0.15.1/{LICENSE,NOTICE}`.
No `typst`/`typst-syntax` crate was present under `~/.cargo/registry/src` to source them from
locally. `LICENSE` is byte-identical to the copy shipped with the local `typst-0.13.1` source
checkout, so the Apache-2.0 text has not changed across these releases. Both carry a `.txt`
extension so that every tool that tracks files by extension carries them.

They cover code ported from Typst 0.15.1 (Apache-2.0). Each file ported from it begins with an
`SPDX-License-Identifier: Apache-2.0` line, the Typst crate and files it comes from, and a note that it is
modified for Austenite; `grep -rl 'SPDX-License-Identifier: Apache-2.0' src` lists them. The crate's own
licence is `Apache-2.0 AND BSD-2-Clause`, so no ported file is offered under BSD-2-Clause alone. Typst's
own copyright holder, "The Typst Project Developers", is not filled into `LICENSE`'s Apache-2.0
appendix; it is taken from the `authors` field of typst's workspace `Cargo.toml`.

Data that is Typst's own or is derived from it is marked the same way where a file can carry a header:
`src/eval/lib/sym_data.rs` tabulates the symbols of `codex` 0.3.0 (Apache-2.0), `src/eval/lib/color_data.rs`
the colour maps of `typst-library`, and `src/eval/lib/model/local.rs` loads the 100 files of
`src/eval/lib/model/translations/*.txt`, which are the `translations/*.txt` files of `typst-library`
unchanged (byte for byte, checked against the 0.15.1 release). Plain data files carry no header of their own, so this
paragraph is their notice. `eval/lib/decimal.rs` follows the algorithms of `rust_decimal` (MIT) in new code
and is Austenite's own; so are `eval/lib/model/{common,lookup}.rs`.

## New Computer Modern Math (`fonts/NewCMMath-Book.otf`, `fonts/NewCMMath-Bold.otf`)

The two faces are Typst's own, unmodified, taken from the `typst-assets` crate at the version the `typst` 0.15.1 release
resolves to (`typst-kit` 0.15.1 requires `typst-assets ^0.15.1`; the release is 0.15.1):

| | |
|---|---|
| Source | `https://static.crates.io/crates/typst-assets/typst-assets-0.15.1.crate`, fetched 2026-10-04, `files/fonts/` |
| Crate sha256 | `bcee505dac6702dd1c5e65aa2e94a6179d19ee09e2e5637d7313db91765dc4e0` |
| `NewCMMath-Book.otf` | 1,432,068 bytes, sha256 `2ea09ebc9167b1e1a66f31390dc917f2d4004ecfca72d51b28010e4ad6becd95` |
| `NewCMMath-Bold.otf` | 1,232,148 bytes, sha256 `c6c0e060da57d4f44274705afb956013047231dc62be5a4b02a351ab0dc43f2f` |
| Licence | GUST Font License 1.0, in `NewCMMath-GUST-LICENSE.txt`, which is the section of that crate's `NOTICE` that covers the New Computer Modern fonts, verbatim |

Each file was checked byte for byte against the copy inside the `typst` 0.15.1 binary (`typst 0.15.1 (9dfd3a08)`), which holds
the Book, Bold and Regular faces. Typst sets an equation in Book (`typst fonts --variants` lists weights 700, 450 and 400, and
a PDF of any equation names `NewCMMath-Book`), so Book is the maths face here and Bold is the face a bold request of the family
resolves to. The Regular face, which Austenite embedded until 2026-10-04, is no longer carried. The faces are not modified, so the
licence's request about renaming derived works does not arise.
