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

# `linkring_vectors.txt`

`linkring/1` signatures produced by `tools/linkring_oracle.py sign`, an
independent Python implementation written from RFC 9496 (ristretto255),
RFC 9380 (`hash_to_ristretto255`) and Bootle et al., ESORICS 2015 (the radix-n
one-out-of-many proof), whose group layer first checks itself against RFC 9496's
published multiples of the generator. No published vectors exist for this
construction, so these stand in for them. `tests/linkring.rs` re-signs each case
in Rust and requires the same ring, digest, tag and body byte for byte, and,
where `python3` is present, also runs the oracle live on fresh random cases.
Written for this crate; no third-party licence applies.

# `ed25519vectors.json`

The C2SP Community Cryptography Test Vectors (CCTV) for Ed25519 verification
edge cases, vendored verbatim from
`https://github.com/C2SP/CCTV/blob/5ea85644bd035c555900a2f707f7e4c31ea65ced/ed25519vectors/ed25519vectors.json`
(SHA-256 `b38e84caf3e7e89170ff520292dbeae421b0a794c27408ce5ce973018fe3d7f9`), the
commit `ed25519-dalek` cites for its own validation tests. Each of the 914
vectors gives a public key, a signature and a message, and flags the edge cases
it exercises: low-order or non-canonical A and R, low-order components, a
low-order residue, a re-encoded k. `tests/ed25519_strict.rs` derives the verdict
a strict verifier owes each vector from its flags alone and asserts it, singly
and in batches.

## Licence

Copyright 2019 Google LLC and 2022 Filippo Valsorda, under the three-clause BSD
licence reproduced in `ed25519vectors_LICENSE.txt`, which travels with the file.
It is included here only as a test oracle and is not part of the compiled
library.
