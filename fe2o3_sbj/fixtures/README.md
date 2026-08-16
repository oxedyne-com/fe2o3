# SBJ conformance fixtures

The teeth of `SPEC.md` §7. Written by `examples/gen_fixtures.rs`, run by `tests/conformance.rs`, and
regenerated rather than patched:

    cargo run -p sbj --example gen_fixtures
    cargo test -p sbj

Each fixture is a directory.

**Acceptance** fixtures carry `doc.jdat`, the document in JDAT text form and the source of truth;
`doc.sbj`, the canonical signed artefact; and `meta.jdat`, what the artefact must turn out to be:
its address, the length of its tree region, its node count and its depth. The suite reads
`doc.jdat`, signs it with the committed key, and requires the bytes it gets back to be `doc.sbj`,
byte for byte.

**Rejection** fixtures carry `doc.sbj`, the bad artefact, and `reject.jdat`, which declares the rule
broken, the step of §2 that must catch it, what the error must say, and the node or the byte it must
name. "It was rejected" is not the claim: the claim is that it was rejected for the right reason.
A rejection fixture also carries `doc.jdat` where the tree region is the encoding of a tree that can
be written down; where the fault is in the bytes themselves, there is no tree to write.

Every rejection fixture past the header is correctly hashed and correctly signed, so that the
rejection can only have come from the rule the fixture breaks, and never from a signature that
happened not to check out.

`key.jdat` holds the fixed key every fixture is signed with, and a second key that signs nothing but the
fixture of a signature by the wrong hand. It is committed on purpose: a fixture signed by a fresh
key would be a different file on every run, and a suite that has to be regenerated to pass tests
nothing. It is a test key, published here, and signs nothing else.

Node labels in `doc.jdat` carry an `sbj_` prefix, because two of the v0 kind labels, `box` and
`list`, are JDAT's own kind labels as well: `(box|{..})` would read back as a `Dat::Box`. None of
this reaches the wire, where BDAT carries the `u16` kind code and no label at all.

The kind code 99 appears in the `unknown_kind` and `unknown_kind_fallback` fixtures. It names no v0
node kind, which is the point of it: the first carries no fallback and is refused, and the second
carries a fallback of known nodes and is accepted (§4.5).

The kind codes 14 (`edit`) and 15 (`surface`) appear in the three `reserved_*` fixtures. They are
not unknown: §4.2 reserves them to the chrome and to applications, and `oxeweb/doc/0` admits the
kinds 1 to 13 and no others. All three are refused, and the third carries a valid fallback and is
refused anyway, which is the point of it: a fallback admits a code the reader has never heard of,
and never one the reader knows a document may not carry.
