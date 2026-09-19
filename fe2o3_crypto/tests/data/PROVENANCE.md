# `ecdsa_verify_tests.txt`

NIST CAVP 186-4 ECDSA `SigVer` (signature verification) test vectors, in the
line-oriented form BoringSSL reformatted them into, vendored here verbatim from
the `ring` crate (`ring-0.17.14/crypto/fipsmodule/ecdsa/ecdsa_verify_tests.txt`).
The file's own header, naming the NIST source, is preserved unchanged.

Each block gives a curve, a public point `X`/`Y`, a message `Digest`, and a
signature `R`/`S`; a block carrying an `Invalid = Y` line is one whose signature
must be rejected. This crate's `tests/p256_verify.rs` reads the `Curve = P-256`
blocks and asserts each verdict.

## Licence

The test data originates in BoringSSL and is used under BoringSSL's permissive
(OpenSSL / ISC-style) licence; `ring` redistributes it on the same terms. It is
included here only as a test oracle and is not part of the compiled library.
