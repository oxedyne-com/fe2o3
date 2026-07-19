//! A self-contained SHA-256 implementation, per FIPS 180-4.
//!
//! This exists because the Web Crypto API offers no SHA3, so a digest agreed between a browser
//! and a Hematite server must be one of the SHA-2 family.

use oxedyne_fe2o3_core::prelude::*;

/// The SHA-256 round constants, being the first thirty two bits of the fractional parts of the
/// cube roots of the first sixty four primes.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The initial hash value, being the first thirty two bits of the fractional parts of the square
/// roots of the first eight primes.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// The SHA-256 block size in bytes.
const BLOCK_LEN: usize = 64;

/// The SHA-256 digest length in bytes.
pub const DIGEST_LEN: usize = 32;

/// An incremental SHA-256 hasher, which buffers input until a full block is available.
#[derive(Clone, Debug)]
pub struct Sha256 {
    /// The running chaining value.
    state:  [u32; 8],
    /// Partial block awaiting compression.
    buf:    [u8; BLOCK_LEN],
    /// Bytes currently held in `buf`.
    buflen: usize,
    /// Total message length in bytes, used for the length padding.
    total:  u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self {
            state:  H0,
            buf:    [0u8; BLOCK_LEN],
            buflen: 0,
            total:  0,
        }
    }
}

impl Sha256 {

    /// Creates a hasher primed with the FIPS 180-4 initial hash value.
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorbs a further slice of the message.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        // Top up any partial block first.
        if self.buflen > 0 {
            let take = std::cmp::min(BLOCK_LEN - self.buflen, data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            data = &data[take..];
            if self.buflen == BLOCK_LEN {
                let block = self.buf;
                self.compress(&block);
                self.buflen = 0;
            }
        }
        // Consume whole blocks directly from the input.
        while data.len() >= BLOCK_LEN {
            let (block, rest) = data.split_at(BLOCK_LEN);
            let mut chunk = [0u8; BLOCK_LEN];
            chunk.copy_from_slice(block);
            self.compress(&chunk);
            data = rest;
        }
        // Retain the remainder.
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buflen = data.len();
        }
    }

    /// Applies the padding and returns the final digest.
    pub fn finish(mut self) -> [u8; DIGEST_LEN] {
        let bitlen = self.total.wrapping_mul(8);
        // A single one bit, then zeroes.  When fewer than eight bytes remain after the one bit the
        // length spills into a further block, which is the case naive implementations get wrong.
        let mut pad = [0u8; 2 * BLOCK_LEN];
        pad[0] = 0x80;
        let padlen = if self.buflen < 56 {
            56 - self.buflen
        } else {
            120 - self.buflen
        };
        pad[padlen..padlen + 8].copy_from_slice(&bitlen.to_be_bytes());
        // `total` is deliberately not corrected here, the hasher being consumed.
        self.update_unlogged(&pad[..padlen + 8]);

        let mut out = [0u8; DIGEST_LEN];
        for (i, word) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// Absorbs padding without counting it toward the message length.
    fn update_unlogged(&mut self, data: &[u8]) {
        let before = self.total;
        self.update(data);
        self.total = before;
    }

    /// Applies the compression function to one sixty four byte block.
    fn compress(&mut self, block: &[u8; BLOCK_LEN]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for i in 0..64 {
            let s1    = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch    = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0    = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj   = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

/// Returns the SHA-256 digest of a single message.
pub fn digest(msg: &[u8]) -> [u8; DIGEST_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(msg);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxedyne_fe2o3_core::{
        byte::B32,
        string::ToHexString,
    };

    /// Renders a digest as lower case hexadecimal, for comparison with the published vectors.
    fn hex(d: [u8; DIGEST_LEN]) -> String {
        B32(d).to_hex_string()
    }

    /// The vectors published with FIPS 180-4 and in the NIST byte oriented test vector set.
    #[test]
    fn test_sha256_fips_180_4_vectors() -> Outcome<()> {
        let vectors: [(&[u8], &str); 3] = [
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (msg, expected) in vectors {
            let d = hex(digest(msg));
            req!(d, expected.to_string(), "SHA-256 of {:?}", msg);
        }
        Ok(())
    }

    /// The two block vector from FIPS 180-4, which exercises the message schedule across a block
    /// boundary.
    #[test]
    fn test_sha256_multi_block() -> Outcome<()> {
        let msg: &[u8] =
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
        let d = hex(digest(msg));
        req!(d, "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1".to_string());
        Ok(())
    }

    /// Lengths either side of the padding boundary, where the length field either fits in the
    /// final block or forces an extra one.  These are the cases that break naive padding.
    #[test]
    fn test_sha256_padding_boundaries() -> Outcome<()> {
        // Digests of 55, 56, 63, 64 and 65 repetitions of 'a'.
        let vectors: [(usize, &str); 5] = [
            (55, "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"),
            (56, "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"),
            (63, "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34"),
            (64, "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"),
            (65, "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0"),
        ];
        for (n, expected) in vectors {
            let msg = vec![b'a'; n];
            let d = hex(digest(&msg));
            req!(d, expected.to_string(), "SHA-256 of {} 'a's", n);
        }
        Ok(())
    }

    /// Streaming in awkwardly sized pieces must agree with hashing in one go, which is what the
    /// `Hasher` impl relies on when it absorbs several input slices.
    #[test]
    fn test_sha256_incremental_matches_oneshot() -> Outcome<()> {
        let msg = vec![b'x'; 1000];
        for chunk in [1usize, 7, 31, 63, 64, 65, 127] {
            let mut hasher = Sha256::new();
            for piece in msg.chunks(chunk) {
                hasher.update(piece);
            }
            // `req!` renders its arguments again on failure, so the digests are bound first.
            let streamed = hex(hasher.finish());
            let oneshot = hex(digest(&msg));
            req!(streamed, oneshot, "chunk size {}", chunk);
        }
        Ok(())
    }

    /// The one million 'a' vector from FIPS 180-4, ignored by default only for its cost.
    #[test]
    #[ignore]
    fn test_sha256_million_a() -> Outcome<()> {
        let mut hasher = Sha256::new();
        let block = [b'a'; 1000];
        for _ in 0..1000 {
            hasher.update(&block);
        }
        let d = hex(hasher.finish());
        req!(d, "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0".to_string());
        Ok(())
    }
}
