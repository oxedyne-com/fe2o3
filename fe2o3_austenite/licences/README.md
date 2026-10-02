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
