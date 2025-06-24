/*!
This module provides an implementation of the [SABER][eprint] key encapsulation and exchange mechanism.

[SABER][saber_web] has been devised by:
- Jan-Pieter D'Anvers, KU Leuven, imec-COSIC
- Angshuman Karmakar, KU Leuven, imec-COSIC
- Sujoy Sinha Roy, KU Leuven, imec-COSIC
- Frederik Vercauteren, KU Leuven, imec-COSIC

[Daan Sprenkels][saber_rust] has made a Rust implementation available, with generics via macros.  This implementation uses const generics and aims to be a little easier to read, test and verify in conjunction with the [C reference implementation][saber_c].

C integer operations were directly translated using the Rust modular arithmetic functions `wrapping_add` and `wrapping_mul`.  However the SABER C reference implementation makes use of unsigned subtraction with implicit coercion.  Subtraction of two unsigned integers automatically triggers C compilers to perform a type coercion (or conversion) to two signed integers that can accomodate any possible result.

```c
#include <stdio.h>
#include <stdint.h>

int main() {
    uint8_t a = 0;
    uint8_t b = 255;
    uint8_t c = a - b;
    printf("%d\n", c); // prints 1
    return 0;
}
```

The equivalent Rust subtraction process must be explicit about these conversions in order to produce the same modular result,

```
let a: u8 = 0;
let b: u8 = 255;
let d: i16 = a as i16 - b as i16;
let c: u8 = d as u8;
println!("The decimal difference {}-{} is {}", a, b, d);
println!("The modular result in decimal difference is -255");
```

[eprint]: https://eprint.iacr.org/2018/230.pdf
[saber_web]: https://www.esat.kuleuven.be/cosic/pqcrypto/saber/
[saber_rust]: https://github.com/dsprenkels/saber-rust
[saber_c]: https://github.com/KULeuven-COSIC/SABER
*/

use crate::generic_saber_api;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    convert::TryInto,
    fmt,
    iter::Chain,
    slice::Iter,
};

use rand_core::{
    OsRng,
    RngCore,
};
use tiny_keccak::{
    Hasher,
    Sha3,
    Shake,
};
use wasm_bindgen::prelude::*;
use zeroize::DefaultIsZeroes;

pub const LIGHTSABER_ID:    u8 = 1;
pub const SABER_ID:         u8 = 2;
pub const FIRESABER_ID:     u8 = 3;

pub const SABER_N:          usize = 256;
pub const SEED_BYTES:       usize = 32;
pub const NOISE_SEED_BYTES: usize = 32;
pub const KEY_BYTES:        usize = 32;
pub const HASH_BYTES:       usize = 32;
pub const EQ:               usize = 13;
pub const EP:               usize = 10;

pub const POLY_BYTES:               usize = EQ * SABER_N / 8;
pub const POLY_COMPRESSED_BYTES:    usize = EP * SABER_N / 8;

// Polynomial multiplication
const MULT_KN: usize = 64;
const MULT_N_SB: usize = SABER_N >> 2; // i.e. SABER_N / 4 = 64
const MULT_N_SB_RES: usize = 2 * MULT_N_SB - 1; // i.e. 127

// Note: we cannot branch on unit structs, nor use them as arguments in functions, so we can't
// switch between them.   You choose one and stick with it, a little like a form of conditional
// compilation.
 
#[wasm_bindgen]
#[derive(Default)]
pub struct LightSaber;

#[wasm_bindgen]
#[derive(Default)]
pub struct Saber;

#[wasm_bindgen]
#[derive(Default)]
pub struct FireSaber;

impl LightSaber { generic_saber_api!(); }
impl Saber      { generic_saber_api!(); }
impl FireSaber  { generic_saber_api!(); }

impl fmt::Display for LightSaber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LightSaber")
    }
}

impl fmt::Display for Saber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Saber")
    }
}

impl fmt::Display for FireSaber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FireSaber")
    }
}

/// Each of the three schemes has important associated constants (i.e. `L`, `ET` and `MU`), a few
/// specific serialisation/deserialisation differences (i.e. `polt2bs` and `bs2polt`) and
/// different algorithms for sampling from a centered binomial distribution (`cbd`).
impl SaberAlgorithm for LightSaber {

    const L: usize = 2;
    const ET: usize = 3;
    const MU: usize = 10;

    fn polt2bs<
        const N: usize,
        const SBK: usize,
    >(
        data: &[u16; N],
        start: usize,
    ) -> [u8; SBK]
    {
        let mut bytes = [0u8; SBK];
    	for j in 0..N/8 {
    		let offset_byte = start + 3 * j;
    		let offset_data = 8 * j;
    		bytes[offset_byte + 0] = (
                (data[offset_data + 0] & 0x7) |
                ((data[offset_data + 1] & 0x7) << 3) | // reminder: "<<" increases the value
                ((data[offset_data + 2] & 0x3) << 6)
            ) as u8;
    		bytes[offset_byte + 1] = (
                ((data[offset_data + 2] >> 2) & 0x01) |
                ((data[offset_data + 3] & 0x7) << 1) |
                ((data[offset_data + 4] & 0x7) << 4) |
                (((data[offset_data + 5]) & 0x01) << 7)
            ) as u8;
    		bytes[offset_byte + 2] = (
                ((data[offset_data + 5] >> 1) & 0x03) |
                ((data[offset_data + 6] & 0x7) << 2) |
                ((data[offset_data + 7] & 0x7) << 5)
            ) as u8;
    	}
        bytes
    }

    fn bs2polt<
        const N: usize,
        const SBK: usize,
    >(
        bytes: &[u8; SBK],
        start: usize,
    ) -> [u16; N]
    {
        let mut data = [0u16; N];

    	for j in 0..N/8 {
	    	let offset_byte = start + 3 * j;
	    	let offset_data = 8 * j;
	    	data[offset_data + 0] = (bytes[offset_byte + 0] & 0x07) as u16;
	    	data[offset_data + 1] = ((bytes[offset_byte + 0] >> 3) & 0x07) as u16;
	    	data[offset_data + 2] = (
                ((bytes[offset_byte + 0] >> 6) & 0x03) |
                ((bytes[offset_byte + 1] & 0x01) << 2)
            ) as u16;
	    	data[offset_data + 3] = ((bytes[offset_byte + 1] >> 1) & 0x07) as u16;
	    	data[offset_data + 4] = ((bytes[offset_byte + 1] >> 4) & 0x07) as u16;
	    	data[offset_data + 5] = (
                ((bytes[offset_byte + 1] >> 7) & 0x01) |
                ((bytes[offset_byte + 2] & 0x03) << 1)
            ) as u16;
	    	data[offset_data + 6] = ((bytes[offset_byte + 2] >> 2) & 0x07) as u16;
	    	data[offset_data + 7] = ((bytes[offset_byte + 2] >> 5) & 0x07) as u16;
	    }
        data
    }

    fn cbd(&self, buf: &[u8]) -> [u16; SABER_N] {
        //msg!("buf len = {} ", buf.len());
        let mut a = [0_u64; 4];
        let mut b = [0_u64; 4];
        let mut secret = [0_16; SABER_N];
    
        for i in 0..SABER_N/4 {
            let k1 = 5 * i;
            let k2 = k1 + 5;
            let t = <Self as SaberAlgorithm>::load_little_endian(&buf[k1..k2]);
            let mut d = 0_u64;
            for j in 0..5 {
                d += (t >> j) & 0x0842108421;
            }
    
            a[0] = d & 0x1f;
            b[0] = (d >> 5) & 0x1f;
            a[1] = (d >> 10) & 0x1f;
            b[1] = (d >> 15) & 0x1f;
            a[2] = (d >> 20) & 0x1f;
            b[2] = (d >> 25) & 0x1f;
            a[3] = (d >> 30) & 0x1f;
            b[3] = d >> 35;

            secret[4 * i + 0] = (a[0] as i64 - b[0] as i64) as u16;
            secret[4 * i + 1] = (a[1] as i64 - b[1] as i64) as u16;
            secret[4 * i + 2] = (a[2] as i64 - b[2] as i64) as u16;
            secret[4 * i + 3] = (a[3] as i64 - b[3] as i64) as u16;
            
        }
        secret
    }

}

impl SaberAlgorithm for Saber {

    const L: usize = 3;
    const ET: usize = 4;
    const MU: usize = 8;

    fn polt2bs<
        const N: usize,
        const SBK: usize,
    >(
        data: &[u16; N],
        start: usize,
    ) -> [u8; SBK]
    {
        let mut bytes = [0u8; SBK];
    	for j in 0..N/2 {
    		let offset_byte = start + j;
    		let offset_data = 2 * j;
    		bytes[offset_byte] = (
                (data[offset_data] & 0x0f) |
                ((data[offset_data + 1] & 0x0f) << 4)
            ) as u8;
    	}
        bytes
    }

    fn bs2polt<
        const N: usize,
        const SBK: usize,
    >(
        bytes: &[u8; SBK],
        start: usize,
    ) -> [u16; N]
    {
        let mut data = [0u16; N];
    	for j in 0..N/2 {
	    	let offset_byte = start + j;
	    	let offset_data = 2 * j;
	    	data[offset_data] = (bytes[offset_byte] & 0x0f) as u16;
	    	data[offset_data + 1] = ((bytes[offset_byte] >> 4) & 0x0f) as u16;
	    }
        data
    }

    fn cbd(&self, buf: &[u8]) -> [u16; SABER_N] {
        let mut a = [0_u32; 4];
        let mut b = [0_u32; 4];
        let mut secret = [0_16; SABER_N];
    
        for i in 0..SABER_N/4 {
            let k1 = 4 * i;
            let k2 = k1 + 4;
            let t = <Self as SaberAlgorithm>::load_little_endian(&buf[k1..k2]) as u32;
            let mut d = 0_u32;
            for j in 0..4 {
                d += (t >> j) & 0x11111111;
            }
    
            a[0] = d & 0xf;
            b[0] = (d >> 4) & 0xf;
            a[1] = (d >> 8) & 0xf;
            b[1] = (d >> 12) & 0xf;
            a[2] = (d >> 16) & 0xf;
            b[2] = (d >> 20) & 0xf;
            a[3] = (d >> 24) & 0xf;
            b[3] = d >> 28;

            secret[4 * i + 0] = (a[0] as i32 - b[0] as i32) as u16;
            secret[4 * i + 1] = (a[1] as i32 - b[1] as i32) as u16;
            secret[4 * i + 2] = (a[2] as i32 - b[2] as i32) as u16;
            secret[4 * i + 3] = (a[3] as i32 - b[3] as i32) as u16;
            
        }
        secret
    }
}

impl SaberAlgorithm for FireSaber {

    const L: usize = 4;
    const ET: usize = 6;
    const MU: usize = 6;

    fn polt2bs<
        const N: usize,
        const SBK: usize,
    >(
        data: &[u16; N],
        start: usize,
    ) -> [u8; SBK]
    {
        let mut bytes = [0u8; SBK];
    	for j in 0..N/4 {
    		let offset_byte = start + 3 * j;
    		let offset_data = 4 * j;
    		bytes[offset_byte + 0] = (
                (data[offset_data + 0] & 0x3f) |
                ((data[offset_data + 1] & 0x03) << 6)
            ) as u8;
    		bytes[offset_byte + 1] = (
                ((data[offset_data + 1] >> 2) & 0x0f) |
                ((data[offset_data + 2] & 0x0f) << 4)
            ) as u8;
    		bytes[offset_byte + 2] = (
                ((data[offset_data + 2] >> 4) & 0x03) |
                ((data[offset_data + 3] & 0x3f) << 2)
            ) as u8;
    	}
        bytes
    }

    fn bs2polt<
        const N: usize,
        const SBK: usize,
    >(
        bytes: &[u8; SBK],
        start: usize,
    ) -> [u16; N]
    {
        let mut data = [0u16; N];
    	for j in 0..N/4 {
		    let offset_byte = start + 3 * j;
		    let offset_data = 4 * j;
		    data[offset_data + 0] = (bytes[offset_byte + 0] & 0x3f) as u16;
		    data[offset_data + 1] = (
                ((bytes[offset_byte + 0] >> 6) & 0x03) |
                ((bytes[offset_byte + 1] & 0x0f) << 2)
            ) as u16;
		    data[offset_data + 2] = (
                ((bytes[offset_byte + 1] & 0xff) >> 4) |
                ((bytes[offset_byte + 2] & 0x03) << 4)
            ) as u16;
		    data[offset_data + 3] = ((bytes[offset_byte + 2] & 0xff) >> 2) as u16;
        }
        data
    }

    fn cbd(&self, buf: &[u8]) -> [u16; SABER_N] {
        let mut a = [0_u32; 4];
        let mut b = [0_u32; 4];
        let mut secret = [0_16; SABER_N];
    
        for i in 0..SABER_N/4 {
            let k1 = 3 * i;
            let k2 = k1 + 3;
            let t = <Self as SaberAlgorithm>::load_little_endian(&buf[k1..k2]) as u32;
            let mut d = 0_u32;
            for j in 0..3 {
                d += (t >> j) & 0x249249;
            }
    
            a[0] = d & 0x7;
            b[0] = (d >> 3) & 0x7;
            a[1] = (d >> 6) & 0x7;
            b[1] = (d >> 9) & 0x7;
            a[2] = (d >> 12) & 0x7;
            b[2] = (d >> 15) & 0x7;
            a[3] = (d >> 18) & 0x7;
            b[3] = d >> 21;
    
            secret[4 * i + 0] = (a[0] as i32 - b[0] as i32) as u16;
            secret[4 * i + 1] = (a[1] as i32 - b[1] as i32) as u16;
            secret[4 * i + 2] = (a[2] as i32 - b[2] as i32) as u16;
            secret[4 * i + 3] = (a[3] as i32 - b[3] as i32) as u16;
            
        }
        secret
    }
}

pub trait SaberAlgorithm: fmt::Display {

    const L: usize;
    const ET: usize;
    const MU: usize;

    const POLY_COIN_BYTES: usize            = Self::MU * SABER_N / 8;
    const POLY_VEC_BYTES: usize             = Self::L * POLY_BYTES;
    const POLY_VEC_COMPRESSED_BYTES: usize  = Self::L * POLY_COMPRESSED_BYTES;
    const SCALE_BYTES_KEM: usize            = Self::ET * SABER_N / 8;
    const INDCPA_PUBLIC_KEY_BYTES: usize    = Self::POLY_VEC_COMPRESSED_BYTES + SEED_BYTES;
    const INDCPA_SECRET_KEY_BYTES: usize    = Self::POLY_VEC_BYTES;
    const PUBLIC_KEY_BYTES: usize           = Self::INDCPA_PUBLIC_KEY_BYTES;
    const SECRET_KEY_BYTES: usize =
        Self::INDCPA_SECRET_KEY_BYTES +
        Self::INDCPA_PUBLIC_KEY_BYTES +
        HASH_BYTES +
        KEY_BYTES;
    const CIPHERTEXT_BYTES: usize =
        Self::POLY_VEC_COMPRESSED_BYTES + Self::SCALE_BYTES_KEM;

    const PK_LEN: usize = Self::POLY_VEC_COMPRESSED_BYTES;
    const SK_LEN: usize = Self::POLY_VEC_BYTES;

    const H1: u16 = (1 << (EQ - EP - 1)) as u16;
    const H2: u16 = (
        (1 << (EP - 2)) -
        (1 << (EP - Self::ET - 1)) +
        (1 << (EQ - EP - 1))
    ) as u16;
    const H3: u16 = (EP - Self::ET) as u16;

    fn pk_len(&self) -> usize { Self::PUBLIC_KEY_BYTES }
    fn sk_len(&self) -> usize { Self::SECRET_KEY_BYTES }
    fn ct_len(&self) -> usize { Self::CIPHERTEXT_BYTES }

    /// IND-CPA Algorithm 17
    /// # Parameters
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `PVB` - The length of the byte string into [`Self::bs2polvecq`] and out of [`Self::polvecq2bs`] (i.e. [`Self::POLY_VEC_BYTES`])
    /// * `L_PVB` - The length of the [`Self::gen_matrix`] byte string (i.e. [`Self::L * Self::POLY_VEC_BYTES`])
    /// * `PVCB` - The length of the byte string from [`Self::polvecp2bs`] (i.e. [`Self::POLY_VEC_COMPRESSED_BYTES`])
    /// * `PCB` - The length of the byte string into cbd (i.e. [`Self::POLY_COIN_BYTES`])
    /// * `L_PCB` - The length of the byte string into bs2polvecq (i.e. [`Self::L * Self::POLY_COIN_BYTES`])
    fn generic_pke_keygen<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
    >(
        &self,
        mut seed_a: [u8; SEED_BYTES],
        seed_s: [u8; NOISE_SEED_BYTES],
    ) -> (
        PublicKey<PVCB>,
        SecretKeyCPA<PVB>,
    ) {
        let mut shake = Shake::v128();
        shake.update(&seed_a);
        shake.finalize(&mut seed_a);

        let mut a = [[[0_u16; SABER_N]; L]; L];
        Self::gen_matrix::<L, PVB, L_PVB>(&mut a, &seed_a);

        let mut secret = [[0_u16; SABER_N]; L];
        Self::gen_secret::<L, PCB, L_PCB>(&self, &mut secret, &seed_s);
        let mut b = [[0_u16; SABER_N]; L];
	    Self::matrix_vector_mul::<L>(&a, &secret, &mut b, true);

	    for i in 0..L {
	    	for j in 0..SABER_N {
	    		b[i][j] = b[i][j].wrapping_add(Self::H1) >> (EQ - EP);
	    	}
	    }

	    let sk = Self::polvecq2bs::<SABER_N, L, POLY_BYTES, PVB>(&secret);
	    let pk = Self::polvecp2bs::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(&b);

        (
            PublicKey::new(seed_a, pk),
            SecretKeyCPA::new(sk),
        )
    }

    /// IND-CPA Algorithm 18
    /// # Parameters
    /// * `L` - [`Self::L`]
    /// * `PVB` - [`Self::POLY_VEC_BYTES`]
    /// * `L_PVB` - [`Self::L * Self::POLY_VEC_BYTES`]
    /// * `PVCB` - [`Self::POLY_VEC_COMPRESSED_BYTES`]
    /// * `PCB` - [`Self::POLY_COIN_BYTES`]
    /// * `L_PCB` - [`Self::L * Self::POLY_COIN_BYTES`]
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    fn generic_pke_enc<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
    >(
        &self,
        m:      &[u8; KEY_BYTES],
        seed_sp:&[u8; SEED_BYTES],
        pk:     &PublicKey<PVCB>,
    ) -> CipherText<SBK, PVCB>
    {
        // 2
        let mut a = [[[0_u16; SABER_N]; L]; L];
        Self::gen_matrix::<L, PVB, L_PVB>(&mut a, pk.seed_ref());
        // 3
        let mut sp = [[0_u16; SABER_N]; L];
        Self::gen_secret::<L, PCB, L_PCB>(&self, &mut sp, seed_sp);
        // 4
        let mut bp = [[0_u16; SABER_N]; L];
	    Self::matrix_vector_mul::<L>(&a, &sp, &mut bp, false);
        // 5-6
	    for i in 0..L {
	    	for j in 0..SABER_N {
	    		bp[i][j] = bp[i][j].wrapping_add(Self::H1) >> (EQ - EP);
	    	}
	    }

        // ?
	    let ct = Self::polvecp2bs::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(&bp);
        // 7
	    let b = Self::bs2polvecp::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(pk.key_ref());
        // 8
        let mut vp = [0_u16; SABER_N];
        Self::inner_prod::<L>(&b, &sp, &mut vp);
        // 9-11
        let mp = Self::bs2polmsg::<SABER_N, KEY_BYTES>(&m);
	    for j in 0..SABER_N {
	    	vp[j] = (
                (vp[j] as i32 - (mp[j] << (EP - 1)) as i32) as u16
            ).wrapping_add(Self::H1) >> (EP - Self::ET);
	    }
        let cm = Self::polt2bs::<SABER_N, SBK>(&vp, 0);

        CipherText {
            cm: cm,
            ct: ct,
        }
    }

    /// A WebAssembly version of Self::generic_pke_enc that avoids accepting a PublicKey argument,
    /// instead accepting the public key and seed parts as arrays, and returns the
    /// ciphertext as an owned vector.
    fn generic_pke_enc_wasm<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
    >(
        &self,
        m:      &[u8; KEY_BYTES],
        seed_sp:&[u8; SEED_BYTES],
        pk_key: &[u8; PVCB],
        pk_seed:&[u8; SEED_BYTES],
    ) -> Vec<u8> // Ciphertext
    {
        // 2
        let mut a = [[[0_u16; SABER_N]; L]; L];
        Self::gen_matrix::<L, PVB, L_PVB>(&mut a, pk_seed);
        // 3
        let mut sp = [[0_u16; SABER_N]; L];
        Self::gen_secret::<L, PCB, L_PCB>(&self, &mut sp, seed_sp);
        // 4
        let mut bp = [[0_u16; SABER_N]; L];
	    Self::matrix_vector_mul::<L>(&a, &sp, &mut bp, false);
        // 5-6
	    for i in 0..L {
	    	for j in 0..SABER_N {
	    		bp[i][j] = bp[i][j].wrapping_add(Self::H1) >> (EQ - EP);
	    	}
	    }

        // ?
	    let ct = Self::polvecp2bs::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(&bp);
        // 7
	    let b = Self::bs2polvecp::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(pk_key);
        // 8
        let mut vp = [0_u16; SABER_N];
        Self::inner_prod::<L>(&b, &sp, &mut vp);
        // 9-11
        let mp = Self::bs2polmsg::<SABER_N, KEY_BYTES>(&m);
	    for j in 0..SABER_N {
	    	vp[j] = (
                (vp[j] as i32 - (mp[j] << (EP - 1)) as i32) as u16
            ).wrapping_add(Self::H1) >> (EP - Self::ET);
	    }
        let cm = Self::polt2bs::<SABER_N, SBK>(&vp, 0);

        [&ct[..], &cm[..]].concat()
    }

    /// IND-CPA Algorithm 19
    /// # Parameters
    /// * `L` - [`Self::L`]
    /// * `PVB` - [`Self::POLY_VEC_BYTES`]
    /// * `L_PVB` - [`Self::L * Self::POLY_VEC_BYTES`]
    /// * `PVCB` - [`Self::POLY_VEC_COMPRESSED_BYTES`]
    /// * `PCB` - [`Self::POLY_COIN_BYTES`]
    /// * `L_PCB` - [`Self::L * Self::POLY_COIN_BYTES`]
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    fn generic_pke_dec<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
    >(
        &self,
        ciphertext: &CipherText<SBK, PVCB>,
        sk: &SecretKeyCPA<PVB>,
    ) -> [u8; KEY_BYTES]
    {

	    let s = Self::bs2polvecq::<SABER_N, L, POLY_BYTES, PVB>(sk.key_ref());
	    let b = Self::bs2polvecp::<SABER_N, L, POLY_COMPRESSED_BYTES, PVCB>(&ciphertext.ct);
        let mut v = [0_u16; SABER_N];
        Self::inner_prod::<L>(&b, &s, &mut v);
	    let cm = Self::bs2polt::<SABER_N, SBK>(&ciphertext.cm, 0);

	    for i in 0..SABER_N {
            //msg!("i = {}", i);
		    v[i] = ((
                v[i].wrapping_add(Self::H2) as i32 - (cm[i] << Self::H3) as i32
            ) >> (EP - 1)) as u16;
	    }

        Self::polmsg2bs::<SABER_N, KEY_BYTES>(&v)
    }

    /// IND-CCA Algorithm 20
    /// # Parameters
    /// * `L` - [`Self::L`]
    /// * `PVB` - [`Self::POLY_VEC_BYTES`]
    /// * `L_PVB` - [`Self::L * Self::POLY_VEC_BYTES`]
    /// * `PVCB` - [`Self::POLY_VEC_COMPRESSED_BYTES`]
    /// * `PCB` - [`Self::POLY_COIN_BYTES`]
    /// * `L_PCB` - [`Self::L * Self::POLY_COIN_BYTES`]
    fn generic_kem_keygen<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
    >(
        &self,
        seed_a: [u8; SEED_BYTES],
        seed_s: [u8; NOISE_SEED_BYTES],
        rand:   [u8; KEY_BYTES],
    ) -> (
        PublicKey<PVCB>,
        SecretKeyCCA<PVB, PVCB>,
    ) {
        let (pk, sk) =
            Self::generic_pke_keygen::<
                L, PVB, L_PVB, PVCB, PCB, L_PCB,
            >(
                &self,
                seed_a,
                seed_s,
            );

        let mut hash_pk = [0_u8; HASH_BYTES];
        let mut sha3 = Sha3::v256();
        sha3.update(&pk.to_bytes());
        sha3.finalize(&mut hash_pk);

        (
            pk.clone(),
            SecretKeyCCA {
                sk:     sk,
                pk:     pk,
                pk_hash:hash_pk,
                rand:   rand,
            },
        )
    }

    /// IND-CCA Algorithm 21
    /// # Parameters
    /// * `L` - [`Self::L`]
    /// * `PVB` - [`Self::POLY_VEC_BYTES`]
    /// * `L_PVB` - [`Self::L * Self::POLY_VEC_BYTES`]
    /// * `PVCB` - [`Self::POLY_VEC_COMPRESSED_BYTES`]
    /// * `PCB` - [`Self::POLY_COIN_BYTES`]
    /// * `L_PCB` - [`Self::L * Self::POLY_COIN_BYTES`]
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    /// * `CT` - [`Self::CIPHERTEXT_BYTES`]
    fn generic_kem_encap<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
        const CT: usize,
    >(
        &self,
        pk: &PublicKey<PVCB>,
        mut m: [u8; KEY_BYTES],
    ) -> (
        [u8; KEY_BYTES], // Session key
        CipherText<SBK, PVCB>,
    ) {
        // 1
        // Accept m as a function parameter to allow validation testing
        // 2
        let mut hasher = Sha3::v256();
        hasher.update(&m);
        hasher.finalize(&mut m);
        // 3
        let mut hash_pk = [0_u8; HASH_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&pk.to_bytes());
        hasher.finalize(&mut hash_pk);
        // 4
        let buf = [&m[..], &hash_pk[..]].concat();
        // 5
        let mut kr = [0u8; 2 * KEY_BYTES];
        let mut hasher = Sha3::v512();
        hasher.update(&buf);
        hasher.finalize(&mut kr);

        // 7
        let ciphertext =
            Self::generic_pke_enc::<
                L, PVB, L_PVB, PVCB, PCB, L_PCB, SBK,
            >(
                &self,
                &m,
                TryInto::<&[u8; SEED_BYTES]>::try_into(&kr[KEY_BYTES..]).unwrap(),
                pk,
            );

        // 8
        let mut rdash = [0_u8; KEY_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&ciphertext.to_bytes::<CT>());
        hasher.finalize(&mut rdash);
        // 9
        let krdash = [&kr[..KEY_BYTES], &rdash[..]].concat();
        // 10
        let mut session_key = [0u8; KEY_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&krdash);
        hasher.finalize(&mut session_key);

        (session_key, ciphertext)
    }

    /// A WebAssembly version of Self::generic_kem_enc that
    /// - avoids accepting a PublicKey argument, instead accepting the public key and seed parts
    /// as a slice and array respectively,
    /// - moves the session secret from an output to a mutable input,
    /// - returns the ciphertext as an owned vector,
    /// - generates the seed value `m` here instead of accepting it as an argument
    fn generic_kem_encap_wasm<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
    >(
        &self,
        pk_key:     &[u8],
        pk_seed:    &[u8],
        mut secret: &mut [u8], // Session key
    ) ->
        Vec<u8> // Ciphertext
    {
        // 1
        let mut m = [0_u8; KEY_BYTES];
        OsRng.fill_bytes(&mut m);
        // 2
        let mut hasher = Sha3::v256();
        hasher.update(&m);
        hasher.finalize(&mut m);
        // 3
        let mut hash_pk = [0_u8; HASH_BYTES];
        let mut hasher = Sha3::v256();
        let pk_bytes = [&pk_key[..], &pk_seed].concat();
        hasher.update(&pk_bytes);
        hasher.finalize(&mut hash_pk);
        // 4
        let buf = [&m[..], &hash_pk[..]].concat();
        // 5
        let mut kr = [0u8; 2 * KEY_BYTES];
        let mut hasher = Sha3::v512();
        hasher.update(&buf);
        hasher.finalize(&mut kr);

        // 7
        let ciphertext =
            Self::generic_pke_enc_wasm::<
                L, PVB, L_PVB, PVCB, PCB, L_PCB, SBK,
            >(
                &self,
                &m,
                TryInto::<&[u8; SEED_BYTES]>::try_into(&kr[KEY_BYTES..]).unwrap(),
                TryInto::<&[u8; PVCB]>::try_into(pk_key).unwrap(),
                TryInto::<&[u8; SEED_BYTES]>::try_into(pk_seed).unwrap(),
            );

        // 8
        let mut rdash = [0_u8; KEY_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&ciphertext);
        hasher.finalize(&mut rdash);
        // 9
        let krdash = [&kr[..KEY_BYTES], &rdash[..]].concat();
        // 10
        let mut hasher = Sha3::v256();
        hasher.update(&krdash);
        hasher.finalize(&mut secret);

        ciphertext
    }

    /// IND-CCA Algorithm 22
    /// # Parameters
    /// * `L` - [`Self::L`]
    /// * `PVB` - [`Self::POLY_VEC_BYTES`]
    /// * `L_PVB` - [`Self::L * Self::POLY_VEC_BYTES`]
    /// * `PVCB` - [`Self::POLY_VEC_COMPRESSED_BYTES`]
    /// * `PCB` - [`Self::POLY_COIN_BYTES`]
    /// * `L_PCB` - [`Self::L * Self::POLY_COIN_BYTES`]
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    /// * `CT` - [`Self::CIPHERTEXT_BYTES`]
    fn generic_kem_decap<
        const L: usize,
        const PVB: usize,
        const L_PVB: usize,
        const PVCB: usize,
        const PCB: usize,
        const L_PCB: usize,
        const SBK: usize,
        const CT: usize,
    >(
        &self,
        ct_bytes:   &[u8],
        sk:         &SecretKeyCCA<PVB, PVCB>,
    ) ->
        Outcome<[u8; KEY_BYTES]> // Session key
    {
        // 1
        let ct = res!(CipherText::<SBK, PVCB>::from_bytes(ct_bytes));
        // 2
        let m = self.generic_pke_dec::<L, PVB, L_PVB, PVCB, PCB, L_PCB, SBK>(&ct, sk.key_ref());
        // 3
        let buf = [&m[..], &sk.pk_hash_ref()[..]].concat();
        // 4
        let mut kr = [0u8; 2 * KEY_BYTES];
        let mut hasher = Sha3::v512();
        hasher.update(&buf);
        hasher.finalize(&mut kr);
        // 6
        let ciphertext_dash =
            Self::generic_pke_enc::<
                L, PVB, L_PVB, PVCB, PCB, L_PCB, SBK,
            >(
                &self,
                &m,
                TryInto::<&[u8; SEED_BYTES]>::try_into(&kr[KEY_BYTES..]).unwrap(),
                sk.pk_ref(),
            );
        let ctdash_bytes = ciphertext_dash.to_bytes::<CT>();
        // 7
        let same = res!(Self::verify(&ctdash_bytes, ct_bytes));
        // 8
        let mut rdash = [0_u8; KEY_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&ctdash_bytes);
        hasher.finalize(&mut rdash);
        // 9-12
        // the order of these concatenations seems to be erroneously reversed in the report text
        let temp = if same {
            [&kr[..KEY_BYTES], &rdash[..]].concat()
        } else {
            [&sk.rand_ref()[..], &rdash[..]].concat()
        };
        // 13
        let mut session_key = [0u8; KEY_BYTES];
        let mut hasher = Sha3::v256();
        hasher.update(&temp);
        hasher.finalize(&mut session_key);
        Ok(session_key)
    }

    fn verify(a: &[u8], b: &[u8]) -> Outcome<bool> {
        if a.len() != b.len() {
            return Err(err!(
                "First slice length = {}, second slice length = {}.",
                a.len(),
                b.len();
            Index, Mismatch));
        }
        let mut r: u64 = 0;
        for i in 0..a.len() {
            r |= (a[i] ^ b[i]) as u64;
        }
        //r = (-r) >> 63;
        Ok(r == 0)
    }

    /// This method represents Algorithm 10 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// packs the 3 significant bits from each u16 polynomial coefficient for the case `N` = `T`
    /// into a byte string.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    fn polt2bs<
        const N: usize,
        const SBK: usize,
    >(
        data: &[u16; N],
        start: usize,
    ) -> [u8; SBK];

    /// This method represents Algorithm 9 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// unpacks chunks of 13 significant bits from the string into u16 polynomial coefficients for
    /// the case `N` = `T`.  This does not change between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the modified polynomial vector (i.e. [`SABER_N`])
    /// * `SBK` - [`Self::SCALE_BYTES_KEM`]
    fn bs2polt<
        const N: usize,
        const SBK: usize,
    >(
        bytes: &[u8; SBK],
        start: usize,
    ) -> [u16; N];

    /// This method represents Algorithm 10 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// packs the 13 significant bits from each u16 polynomial coefficient for the case `N` = `q`
    /// into a byte string.  This does not change between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    fn polq2bs<
        const N: usize,
    >(
        data: &[u16; N],
        bytes: &mut [u8],
        start: usize,
    ) {
    	for j in 0..N/8 {
		    let offset_byte = start + 13 * j;
		    let offset_data = 8 * j;
		    bytes[offset_byte + 0] = (data[offset_data + 0] & (0xff)) as u8;
		    bytes[offset_byte + 1] = (
                ((data[offset_data + 0] >> 8) & 0x1f) |
                ((data[offset_data + 1] & 0x07) << 5)
            ) as u8;
		    bytes[offset_byte + 2] = ((data[offset_data + 1] >> 3) & 0xff) as u8;
		    bytes[offset_byte + 3] = (
                ((data[offset_data + 1] >> 11) & 0x03) |
                ((data[offset_data + 2] & 0x3f) << 2)
            ) as u8;
		    bytes[offset_byte + 4] = (
                ((data[offset_data + 2] >> 6) & 0x7f) |
                ((data[offset_data + 3] & 0x01) << 7)
            ) as u8;
		    bytes[offset_byte + 5] = ((data[offset_data + 3] >> 1) & 0xff) as u8;
		    bytes[offset_byte + 6] = (
                ((data[offset_data + 3] >> 9) & 0x0f) |
                ((data[offset_data + 4] & 0x0f) << 4)
            ) as u8;
		    bytes[offset_byte + 7] = ((data[offset_data + 4] >> 4) & 0xff) as u8;
		    bytes[offset_byte + 8] = (
                ((data[offset_data + 4] >> 12) & 0x01) |
                ((data[offset_data + 5] & 0x7f) << 1)
            ) as u8;
		    bytes[offset_byte + 9] = (
                ((data[offset_data + 5] >> 7) & 0x3f) |
                ((data[offset_data + 6] & 0x03) << 6)
            ) as u8;
		    bytes[offset_byte + 10] = ((data[offset_data + 6] >> 2) & 0xff) as u8;
		    bytes[offset_byte + 11] = (
                ((data[offset_data + 6] >> 10) & 0x07) |
                ((data[offset_data + 7] & 0x1f) << 3)
            ) as u8;
		    bytes[offset_byte + 12] = ((data[offset_data + 7] >> 5) & 0xff) as u8;
        }
    }

    /// This method represents Algorithm 12 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// packs the 13 significant bits from each u16 polynomial coefficient for the given vector of
    /// polynomials, where `N` = `q`, into a byte string.  This does not change between the
    /// schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `I` - The length of the byte string from polq2bs (i.e. [`POLY_BYTES`])
    /// * `B` - The length of the outgoing byte string (i.e. [`Self::POLY_VEC_BYTES`])
    fn polvecq2bs<
        const N: usize,
        const L: usize,
        const I: usize,
        const B: usize,
    >(
        data: &[[u16; N]; L],
    ) -> [u8; B]
    {
        let mut bytes = [0; B];
    	for i in 0..L {
            Self::polq2bs(&data[i], &mut bytes, i * I);
    	}
        bytes
    }

    /// This method represents Algorithm 10 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// packs the 10 significant bits from each u16 polynomial coefficient for the case `N` = `p`
    /// into a byte string.  This does not change between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the incoming polynomial vector (i.e. [`SABER_N`])
    fn polp2bs<
        const N: usize,
    >(
        data: &[u16; N],
        bytes: &mut [u8],
        start: usize,
    ) {
    	for j in 0..N/4 {
		    let offset_byte = start + 5 * j;
		    let offset_data = 4 * j;
		    bytes[offset_byte + 0] = (data[offset_data + 0] & (0xff)) as u8;
		    bytes[offset_byte + 1] = (
                ((data[offset_data + 0] >> 8) & 0x03) |
                ((data[offset_data + 1] & 0x3f) << 2)
            ) as u8;
		    bytes[offset_byte + 2] = (
                ((data[offset_data + 1] >> 6) & 0x0f) |
                ((data[offset_data + 2] & 0x0f) << 4)
            ) as u8;
		    bytes[offset_byte + 3] = (
                ((data[offset_data + 2] >> 4) & 0x3f) |
                ((data[offset_data + 3] & 0x03) << 6)
            ) as u8;
		    bytes[offset_byte + 4] = ((data[offset_data + 3] >> 2) & 0xff) as u8;
	    }
    }

    /// This method represents Algorithm 12 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// packs the 10 significant bits from each u16 polynomial coefficient for the given vector of
    /// polynomials, where `N` = `p`, into a byte string.  This does not change between the
    /// schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the incoming polynomial vector (i.e. [`SABER_N`])
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `I` - The length of the byte string from polq2bs (i.e. [`POLY_COMPRESSED_BYTES`])
    /// * `B` - The length of the outgoing byte string (i.e. [`Self::POLY_VEC_COMPRESSED_BYTES`])
    fn polvecp2bs<
        const N: usize,
        const L: usize,
        const I: usize,
        const B: usize,
    >(
        data: &[[u16; N]; L],
    ) -> [u8; B]
    {
        let mut bytes = [0; B];
    	for i in 0..L {
            Self::polp2bs(&data[i], &mut bytes, i * I);
    	}
        bytes
    }

    /// This method represents Algorithm 9 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// unpacks chunks of 13 significant bits from the string into u16 polynomial coefficients for
    /// the case `N` = `q`.  This does not change between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the modified polynomial vector (i.e. [`SABER_N`])
    fn bs2polq<
        const N: usize,
    >(
        bytes: &[u8],
        data: &mut [u16; N],
    ) {
    	for j in 0..N/8 {
    		let offset_byte = 13 * j;
    		let offset_data = 8 * j;
    		data[offset_data + 0] =
                (bytes[offset_byte + 0] as u16 & (0xff)) |
                ((bytes[offset_byte + 1] as u16 & 0x1f) << 8);
    		data[offset_data + 1] =
                (bytes[offset_byte + 1] as u16 >> 5 & (0x07)) |
                ((bytes[offset_byte + 2] as u16 & 0xff) << 3) |
                ((bytes[offset_byte + 3] as u16 & 0x03) << 11);
    		data[offset_data + 2] =
                (bytes[offset_byte + 3] as u16 >> 2 & (0x3f)) |
                ((bytes[offset_byte + 4] as u16 & 0x7f) << 6);
    		data[offset_data + 3] =
                (bytes[offset_byte + 4] as u16 >> 7 & (0x01)) |
                ((bytes[offset_byte + 5] as u16 & 0xff) << 1) |
                ((bytes[offset_byte + 6] as u16 & 0x0f) << 9);
    		data[offset_data + 4] =
                (bytes[offset_byte + 6] as u16 >> 4 & (0x0f)) |
                ((bytes[offset_byte + 7] as u16 & 0xff) << 4) |
                ((bytes[offset_byte + 8] as u16 & 0x01) << 12);
    		data[offset_data + 5] =
                (bytes[offset_byte + 8] as u16 >> 1 & (0x7f)) |
                ((bytes[offset_byte + 9] as u16 & 0x3f) << 7);
    		data[offset_data + 6] =
                (bytes[offset_byte + 9] as u16 >> 6 & (0x03)) |
                ((bytes[offset_byte + 10] as u16 & 0xff) << 2) |
                ((bytes[offset_byte + 11] as u16 & 0x07) << 10);
    		data[offset_data + 7] =
                (bytes[offset_byte + 11] as u16 >> 3 & (0x1f)) |
                ((bytes[offset_byte + 12] as u16 & 0xff) << 5);
    	}
    }

    /// This method represents Algorithm 11 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// unpacks chunks of 13 significant bits from the string into each set of u16 coefficients for
    /// the newly created vector of polynomials for the case `N` = `q`.  This does not change
    /// between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `I` - The length of the byte string to bs2polq (i.e. [`POLY_BYTES`])
    /// * `B` - The length of the outgoing byte string (i.e. [`Self::POLY_VEC_BYTES`])
    fn bs2polvecq<
        const N: usize,
        const L: usize,
        const I: usize,
        const B: usize,
    >(
        bytes: &[u8; B],
    ) -> [[u16; N]; L]
    {
        let mut data = [[0; N]; L];
    	for i in 0..L {
            let j = i * I;
            Self::bs2polq(&bytes[j..j+I], &mut data[i]);
    	}
        data
    }

    /// This method represents Algorithm 9 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// unpacks chunks of 10 significant bits from the string into u16 polynomial coefficients for
    /// the case `N` = `p`.  This does not change between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the modified polynomial vector (i.e. [`SABER_N`])
    fn bs2polp<
        const N: usize,
    >(
        bytes: &[u8],
        data: &mut [u16; N],
    ) {
    	for j in 0..N/4 {
		    let offset_byte = 5 * j;
		    let offset_data = 4 * j;
		    data[offset_data + 0] = 
                (bytes[offset_byte + 0] as u16 & (0xff)) |
                ((bytes[offset_byte + 1] as u16 & 0x03) << 8);
		    data[offset_data + 1] =
                ((bytes[offset_byte + 1] as u16 >> 2) & (0x3f)) |
                ((bytes[offset_byte + 2] as u16 & 0x0f) << 6);
		    data[offset_data + 2] =
                ((bytes[offset_byte + 2] as u16 >> 4) & (0x0f)) |
                ((bytes[offset_byte + 3] as u16 & 0x3f) << 4);
		    data[offset_data + 3] =
                ((bytes[offset_byte + 3] as u16 >> 6) & (0x03)) |
                ((bytes[offset_byte + 4] as u16 & 0xff) << 2);
        }
    }

    /// This method represents Algorithm 11 of the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// unpacks chunks of 13 significant bits from the string into each set of u16 coefficients for
    /// the newly created vector of polynomials for the case `N` = `q`.  This does not change
    /// between the schemes.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `I` - The length of the byte string to bs2polq (i.e. [`POLY_COMPRESSED_BYTES`])
    /// * `B` - The length of the outgoing byte string (i.e. [`Self::POLY_VEC_COMPRESSED_BYTES`])
    fn bs2polvecp<
        const N: usize,
        const L: usize,
        const I: usize,
        const B: usize,
    >(
        bytes: &[u8; B],
    ) -> [[u16; N]; L]
    {
        let mut data = [[0; N]; L];
    	for i in 0..L {
            let j = i * I;
            Self::bs2polp(&bytes[j..j+I], &mut data[i]);
    	}
        data
    }

    /// This method is not explicitly documented in the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// interprets the bits from an arbitrary message as polynomial coefficients.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `B` - The length of the incoming byte string (i.e. [`KEY_BYTES`])
    fn bs2polmsg<
        const N: usize,
        const B: usize,
    >(
        bytes: &[u8; B],
    ) -> [u16; N]
    {
        let mut data = [0; N];
    	for j in 0..B {
            for i in 0..8 {
                data[j * 8 + i] = ((bytes[j] as u16) >> i) & 0x01;
            }
    	}
        data
    }

    /// This method is not explicitly documented in the
    /// [spec](https://www.esat.kuleuven.be/cosic/pqcrypto/saber/files/saberspecround3.pdf).  It
    /// interprets polynomial coefficients as a byte string message.
    ///
    /// # Parameters
    /// * `N` - The order of the outgoing polynomial vector (i.e. [`SABER_N`])
    /// * `B` - The length of the incoming byte string (i.e. [`KEY_BYTES`])
    fn polmsg2bs<
        const N: usize,
        const B: usize,
    >(
        data: &[u16; N],
    ) -> [u8; B]
    {
        let mut bytes = [0; B];
    	for j in 0..B {
            for i in 0..8 {
                bytes[j] = bytes[j] | (((data[j * 8 + i] & 0x01) << i) as u8);
            }
    	}
        bytes
    }
    
    /// Performs Karatsuba multiplication of a subset of coefficients from two polynomials.
    fn karatsuba_simple(
        a_1: &[u16; MULT_KN],
        b_1: &[u16; MULT_KN],
    ) -> [u16; 2 * MULT_KN - 1]
    {
        let mut d01 = [0_i32; MULT_KN / 2 - 1];
        let mut d0123 = [0_i32; MULT_KN / 2 - 1];
        let mut d23 = [0_i32; MULT_KN / 2 - 1];
        let mut result_d01 = [0_i32; MULT_KN - 1];
        let mut result_final = [0_u16; 2 * MULT_KN - 1];
    
        for i in 0..MULT_KN/4 {
            let acc1 = a_1[i]; //a0
            let acc2 = a_1[i + MULT_KN / 4]; //a1
            let acc3 = a_1[i + 2 * MULT_KN / 4]; //a2
            let acc4 = a_1[i + 3 * MULT_KN / 4]; //a3

            for j in 0..MULT_KN/4 {
    
                let mut acc5 = b_1[j]; //b0
                let mut acc6 = b_1[j + MULT_KN / 4]; //b1
    
                result_final[i + j + 0 * MULT_KN / 4] =
                    result_final[i + j + 0 * MULT_KN / 4]
                    .wrapping_add(acc1.wrapping_mul(acc5));
                result_final[i + j + 2 * MULT_KN / 4] =
                    result_final[i + j + 2 * MULT_KN / 4]
                    .wrapping_add(acc2.wrapping_mul(acc6));
    
                let mut acc7 = acc5.wrapping_add(acc6); //b01
                let mut acc8 = acc1.wrapping_add(acc2); //a01
                d01[i + j] = d01[i + j].wrapping_add(acc7.wrapping_mul(acc8) as i32);
    
                acc7 = b_1[j + 2 * MULT_KN / 4]; //b2
                acc8 = b_1[j + 3 * MULT_KN / 4]; //b3
                result_final[i + j + 4 * MULT_KN / 4] =
                    result_final[i + j + 4 * MULT_KN / 4]
                    .wrapping_add(acc7.wrapping_mul(acc3));
    
                result_final[i + j + 6 * MULT_KN / 4] =
                    result_final[i + j + 6 * MULT_KN / 4]
                    .wrapping_add(acc8.wrapping_mul(acc4));
    
                let acc9 = acc3.wrapping_add(acc4);
                let acc10 = acc7.wrapping_add(acc8);
                d23[i + j] = d23[i + j].wrapping_add(acc9.wrapping_mul(acc10) as i32);
    
                acc5 = acc5.wrapping_add(acc7); //b02
                acc7 = acc1.wrapping_add(acc3); //a02
                result_d01[i + j + 0 * MULT_KN / 4] =
                    result_d01[i + j + 0 * MULT_KN / 4]
                    .wrapping_add(acc5.wrapping_mul(acc7) as i32);
    
                acc6 = acc6.wrapping_add(acc8); //b13
                acc8 = acc2.wrapping_add(acc4);
                result_d01[i + j + 2 * MULT_KN / 4] =
                    result_d01[i + j + 2 * MULT_KN / 4]
                    .wrapping_add(acc6.wrapping_mul(acc8) as i32);
    
                acc5 = acc5.wrapping_add(acc6);
                acc7 = acc7.wrapping_add(acc8);
                d0123[i + j] = d0123[i + j].wrapping_add(acc5.wrapping_mul(acc7) as i32);
            }
        }
    
        // 2nd last stage
    
        for i in 0..(MULT_KN/2)-1 {
            d0123[i] = d0123[i]
                - result_d01[i + 0 * MULT_KN / 4]
                - result_d01[i + 2 * MULT_KN / 4];
            d01[i] = d01[i]
                - (result_final[i + 0 * MULT_KN / 4] as i32)
                - (result_final[i + 2 * MULT_KN / 4] as i32);
            d23[i] = d23[i] 
                - (result_final[i + 4 * MULT_KN / 4] as i32)
                - (result_final[i + 6 * MULT_KN / 4] as i32);
        }
    
        for i in 0..(MULT_KN/2)-1 {
            result_d01[i + 1 * MULT_KN / 4] =
                result_d01[i + 1 * MULT_KN / 4].wrapping_add(d0123[i]);
            result_final[i + 1 * MULT_KN / 4] =
                result_final[i + 1 * MULT_KN / 4].wrapping_add(d01[i] as u16);
            result_final[i + 5 * MULT_KN / 4] =
                result_final[i + 5 * MULT_KN / 4].wrapping_add(d23[i] as u16);
        }
    
        // Last stage
        for i in 0..MULT_KN-1 {
            result_d01[i] = result_d01[i]
                - (result_final[i] as i32)
                - (result_final[i + MULT_KN] as i32);
        }
    
        for i in 0..MULT_KN-1 {
            result_final[i + 1 * MULT_KN / 2] =
                result_final[i + 1 * MULT_KN / 2].wrapping_add(result_d01[i] as u16);
        }
        result_final
    }

    fn toom_cook_4way(
        a: &[u16; SABER_N],
        b: &[u16; SABER_N],
    ) -> [u16; 2 * SABER_N]
    {
        const INV3: i32 = 43691;
        const INV9: i32 = 36409;
        const INV15: i32 = 61167;
    
        let mut aw1 = [0_u16; MULT_N_SB];
        let mut aw2 = [0_u16; MULT_N_SB];
        let mut aw3 = [0_u16; MULT_N_SB];
        let mut aw4 = [0_u16; MULT_N_SB];
        let mut aw5 = [0_u16; MULT_N_SB];
        let mut aw6 = [0_u16; MULT_N_SB];
        let mut aw7 = [0_u16; MULT_N_SB];

        let mut bw1 = [0_u16; MULT_N_SB];
        let mut bw2 = [0_u16; MULT_N_SB];
        let mut bw3 = [0_u16; MULT_N_SB];
        let mut bw4 = [0_u16; MULT_N_SB];
        let mut bw5 = [0_u16; MULT_N_SB];
        let mut bw6 = [0_u16; MULT_N_SB];
        let mut bw7 = [0_u16; MULT_N_SB];

        let a0 = &a;
        let a1 = &a[MULT_N_SB..];
        let a2 = &a[2 * MULT_N_SB..];
        let a3 = &a[3 * MULT_N_SB..];
        let b0 = &b;
        let b1 = &b[MULT_N_SB..];
        let b2 = &b[2 * MULT_N_SB..];
        let b3 = &b[3 * MULT_N_SB..];
    
        let mut c = [0_u16; 2 * SABER_N];
    
        // EVALUATION
        for j in 0..MULT_N_SB {
            let r0 = a0[j];
            let r1 = a1[j];
            let r2 = a2[j];
            let r3 = a3[j];
            let mut r4 = r0.wrapping_add(r2);
            let mut r5 = r1.wrapping_add(r3);
            let mut r6 = r4.wrapping_add(r5);
            let mut r7 = ((r4 as i32) - (r5 as i32)) as u16;
            aw3[j] = r6;
            aw4[j] = r7;
            r4 = (r0 << 2).wrapping_add(r2) << 1;
            r5 = (r1 << 2).wrapping_add(r3);
            r6 = r4.wrapping_add(r5);
            r7 = ((r4 as i32) - (r5 as i32)) as u16;
            aw5[j] = r6;
            aw6[j] = r7;
            r4 = (r3 << 3).wrapping_add(r2 << 2).wrapping_add(r1 << 1).wrapping_add(r0);
            aw2[j] = r4;
            aw7[j] = r0;
            aw1[j] = r3;
        }

        for j in 0..MULT_N_SB {
            let r0 = b0[j];
            let r1 = b1[j];
            let r2 = b2[j];
            let r3 = b3[j];
            let mut r4 = r0.wrapping_add(r2);
            let mut r5 = r1.wrapping_add(r3);
            let mut r6 = r4.wrapping_add(r5);
            let mut r7 = ((r4 as i32) - (r5 as i32)) as u16;
            bw3[j] = r6;
            bw4[j] = r7;
            r4 = (r0 << 2).wrapping_add(r2) << 1;
            r5 = (r1 << 2).wrapping_add(r3);
            r6 = r4.wrapping_add(r5);
            r7 = ((r4 as i32) - (r5 as i32)) as u16;
            bw5[j] = r6;
            bw6[j] = r7;
            r4 = (r3 << 3).wrapping_add(r2 << 2).wrapping_add(r1 << 1).wrapping_add(r0);
            bw2[j] = r4;
            bw7[j] = r0;
            bw1[j] = r3;
        }
    
        // MULTIPLICATION
        let w1 = Self::karatsuba_simple(&aw1, &bw1);
        let w2 = Self::karatsuba_simple(&aw2, &bw2);
        let w3 = Self::karatsuba_simple(&aw3, &bw3);
        let w4 = Self::karatsuba_simple(&aw4, &bw4);
        let w5 = Self::karatsuba_simple(&aw5, &bw5);
        let w6 = Self::karatsuba_simple(&aw6, &bw6);
        let w7 = Self::karatsuba_simple(&aw7, &bw7);
    
        // INTERPOLATION
        for i in 0..MULT_N_SB_RES {
            let     r0 = w1[i] as i32;
            let mut r1 = w2[i] as i32;
            let mut r2 = w3[i] as i32;
            let mut r3 = w4[i] as i32;
            let mut r4 = w5[i] as i32;
            let mut r5 = w6[i] as i32;
            let     r6 = w7[i] as i32;
    
            r1 = r1 + r4;
            r1 &= 0xffff;
            r5 = r5 - r4;
            r5 &= 0xffff;
            r3 = (r3 - r2) >> 1;
            r3 &= 0xffff;
            r4 = r4 - r0;
            r4 &= 0xffff;
            r4 = r4 - (r6 << 6);
            r4 &= 0xffff;
            r4 = (r4 << 1) + r5;
            r4 &= 0xffff;
            r2 = r2 + r3;
            r2 &= 0xffff;
            r1 = r1 - (r2 << 6) - r2;
            r1 &= 0xffff;
            r2 = r2 - r6;
            r2 &= 0xffff;
            r2 = r2 - r0;
            r2 &= 0xffff;
            r1 = r1 + 45 * r2;
            r1 &= 0xffff;
            r4 = (r4 - (r2 << 3)).wrapping_mul(INV3) >> 3;
            r4 &= 0xffff;
            r5 = r5 + r1;
            r5 &= 0xffff;
            r1 = (r1 + (r3 << 4)).wrapping_mul(INV9) >> 1;
            r1 &= 0xffff;
            r3 = -(r3 + r1);
            r3 &= 0xffff;
            r5 = (30 * r1 - r5).wrapping_mul(INV15) >> 2;
            r5 &= 0xffff;
            r2 = r2 - r4;
            r2 &= 0xffff;
            r1 = r1 - r5;
            r1 &= 0xffff;

            c[i]        = c[i].wrapping_add(r6 as u16);
            c[i + 64]   = c[i + 64].wrapping_add(r5 as u16);
            c[i + 128]  = c[i + 128].wrapping_add(r4 as u16);
            c[i + 192]  = c[i + 192].wrapping_add(r3 as u16);
            c[i + 256]  = c[i + 256].wrapping_add(r2 as u16);
            c[i + 320]  = c[i + 320].wrapping_add(r1 as u16);
            c[i + 384]  = c[i + 384].wrapping_add(r0 as u16);
        }
        c
    }
    
    /// Outlined in 8.3.7
    fn poly_mul_acc(
        a: &[u16; SABER_N],
        b: &[u16; SABER_N],
        res: &mut [u16; SABER_N],
    ) {
    	let c = Self::toom_cook_4way(a, b);
    
    	for i in SABER_N..2*SABER_N {
    		res[i - SABER_N] = (
                res[i - SABER_N].wrapping_add(c[i - SABER_N]) as i32 -
                c[i] as i32
            ) as u16;
    	}
    }

    /// Algorithm 13
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    fn matrix_vector_mul<
        const L: usize,
    >(
        a:      &[[[u16; SABER_N]; L]; L],
        s:      &[[u16; SABER_N]; L],
        res:    &mut [[u16; SABER_N]; L],
        transpose: bool,
    ) {
    	for i in 0..L {
    		for j in 0..L {
    			if transpose {
    				Self::poly_mul_acc(&a[j][i], &s[j], &mut res[i]);
    			} else {
    				Self::poly_mul_acc(&a[i][j], &s[j], &mut res[i]);
    			}	
    		}
    	}
    }
    
    /// Algorithm 14
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    fn inner_prod<
        const L: usize,
    >(
        b:      &[[u16; SABER_N]; L],
        s:      &[[u16; SABER_N]; L],
        res:    &mut [u16; SABER_N],
    ) {
    	for j in 0..L {
    		Self::poly_mul_acc(&b[j], &s[j], res);
    	}
    }
    
    /// Algorithm 15
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `P` - The length of the byte string into bs2polvecq (i.e. [`Self::POLY_VEC_BYTES`])
    /// * `B` - The length of the initial byte string (i.e. [`Self::L * Self::POLY_VEC_BYTES`])
    fn gen_matrix<
        const L: usize,
        const P: usize,
        const B: usize,
    >(
        a:      &mut [[[u16; SABER_N]; L]; L],
        seed:   &[u8; SEED_BYTES],
    ) {
    	let mut buf = [0_u8; B];
        let mut shake = Shake::v128();
        shake.update(&seed[..]);
        shake.finalize(&mut buf);
    
    	for i in 0..L {
            let k = i*P;
    		a[i] = Self::bs2polvecq::<
                SABER_N,
                L,
                POLY_BYTES,
                P,
            >(
                TryInto::<&[u8; P]>::try_into(&buf[k..k+P]).unwrap(),
            );
    	}
    }

    
    /// Centered Binomial Distribution
    fn cbd(&self, buf: &[u8]) -> [u16; SABER_N];

    /// Support function for [`cbd`].
    fn load_little_endian(
        x: &[u8],
    ) -> u64
    {
        let mut r = x[0] as u64;
        for i in 1..x.len() {
            r |= (x[i] as u64) << (8 * i);
        }
        r
    }

    /// Algorithm 16
    /// * `L` - The length of the polynomial vector (i.e. the rank [`Self::L`])
    /// * `PCB` - The length of the byte string into cbd (i.e. [`Self::POLY_COIN_BYTES`])
    /// * `L_PCB` - The length of the byte string into bs2polvecq (i.e. [`Self::L * Self::POLY_COIN_BYTES`])
    fn gen_secret<
        const L: usize,
        const PCB: usize,
        const L_PCB: usize,
    >(
        &self,
        s:      &mut [[u16; SABER_N]; L],
        seed:   &[u8; NOISE_SEED_BYTES],
    ) {
    	let mut buf = [0_u8; L_PCB];
        let mut shake = Shake::v128();
        shake.update(&seed[..]);
        shake.finalize(&mut buf);
    
    	for i in 0..L {
            let k = i*PCB;
    		s[i] = self.cbd(&buf[k..k+PCB]);
    	}
    }
    
}

/// The public key used in both kex and kem.
#[derive(Clone, Copy)]
pub struct PublicKey<const LEN: usize> {
    seed: [u8; SEED_BYTES],
    key:  [u8; LEN],
}

// For zeroize
impl<const LEN: usize> Default for PublicKey<LEN> {
    fn default() -> Self {
        Self {
            seed:   [0; SEED_BYTES],
            key:    [0; LEN],
        }
    }
}

impl<const LEN: usize> PublicKey<LEN> {

    fn new(seed: [u8; SEED_BYTES], key: [u8; LEN]) -> Self {
        Self {
            seed:   seed,
            key:    key,
        }
    }

    fn key(self) -> [u8; LEN] {
        self.key
    }

    fn key_ref(&self) -> &[u8; LEN] {
        &self.key
    }

    fn seed(self) -> [u8; SEED_BYTES] {
        self.seed
    }

    fn seed_ref(&self) -> &[u8; SEED_BYTES] {
        &self.seed
    }

    fn iter(&self) -> Chain<Iter<u8>, Iter<u8>> {
        self.key.iter().chain(self.seed.iter())
    }

    pub fn to_bytes(&self) -> Vec<u8> { 
        [&self.key[..], &self.seed[..]].concat()
    }

    pub fn from_bytes(b: &[u8]) -> Outcome<Self> {
        let end: usize = LEN;
        let key = res!(
            TryInto::<&[u8; LEN]>::try_into(&b[..end]),
            Conversion, Bytes,
        );
        let seed = res!(
            TryInto::<&[u8; SEED_BYTES]>::try_into(&b[end..]),
            Conversion, Bytes,
        );
        Ok( PublicKey {
            seed:   *seed,
            key:    *key,
        })
    }

    pub fn byte_len(&self) -> usize {
        SEED_BYTES + LEN
    }
}

/// The key used in the key exchange mechanism (kex).
#[derive(Clone, Copy)]
pub struct SecretKeyCPA<const LEN: usize> {
    key: [u8; LEN],
}

// For zeroize
impl<const LEN: usize> Default for SecretKeyCPA<LEN> {
    fn default() -> Self {
        Self {
            key: [0; LEN],
        }
    }
}

impl<const LEN: usize> SecretKeyCPA<LEN> {

    fn new(key: [u8; LEN]) -> Self {
        Self {
            key: key,
        }
    }

    fn key(self) -> [u8; LEN] {
        self.key
    }

    fn key_ref(&self) -> &[u8; LEN] {
        &self.key
    }

    fn iter(&self) -> Iter<u8> {
        self.key.iter()
    }

    fn from_bytes(b: &[u8]) -> Outcome<Self> {
        let key = res!(
            TryInto::<&[u8; LEN]>::try_into(&b[..]),
            Conversion, Bytes,
        );
        Ok( SecretKeyCPA {
            key:    *key,
        })
    }

    fn byte_len(&self) -> usize {
        SEED_BYTES
    }
}

/// The key used in the key encaspulation mechanism (kem).
#[derive(Clone, Copy, Default)]
pub struct SecretKeyCCA<
    const SK_LEN: usize,
    const PK_LEN: usize,
> {
    sk:     SecretKeyCPA<SK_LEN>,
    pk:     PublicKey<PK_LEN>,
    pk_hash:[u8; HASH_BYTES],
    rand:   [u8; KEY_BYTES],
}

impl<
    const SK_LEN: usize,
    const PK_LEN: usize,
>
    DefaultIsZeroes for SecretKeyCCA<SK_LEN, PK_LEN>
{}

impl<
    const SK_LEN: usize,
    const PK_LEN: usize,
>
    SecretKeyCCA<SK_LEN, PK_LEN>
{

    fn key(self) -> SecretKeyCPA<SK_LEN> {
        self.sk
    }

    fn key_ref(&self) -> &SecretKeyCPA<SK_LEN> {
        &self.sk
    }

    fn pk_hash_ref(&self) -> &[u8; HASH_BYTES] {
        &self.pk_hash
    }

    fn rand_ref(&self) -> &[u8; KEY_BYTES] {
        &self.rand
    }

    fn pk_ref(&self) -> &PublicKey<PK_LEN> {
        &self.pk
    }

    fn iter(&self) ->
        Chain<
            Chain<
                Chain<
                    Iter<u8>,
                    Chain<
                        Iter<u8>,
                        Iter<u8>,
                    >,
                >,
                Iter<u8>,
            >,
            Iter<u8>,
        >
    {
        self.sk.iter()
            .chain(self.pk.iter())
            .chain(self.pk_hash.iter())
            .chain(self.rand.iter())
    }

    pub fn to_bytes(&self) -> Vec<u8> { 
        [
            &self.sk.key()[..],
            &self.pk.to_bytes(),
            &self.pk_hash[..],
            &self.rand[..],
        ].concat()
    }

    pub fn from_bytes(b: &[u8]) -> Outcome<Self> {
        let mut end: usize = KEY_BYTES;
        let rand = res!(
            TryInto::<&[u8; KEY_BYTES]>::try_into(&b[..end]),
            Conversion, Bytes,
        );
        let mut start = end;
        end = start + HASH_BYTES;
        let hash_pk = res!(
            TryInto::<&[u8; HASH_BYTES]>::try_into(&b[start..end]),
            Conversion, Bytes,
        );
        start = end;
        end = start + PK_LEN + SEED_BYTES;
        let pk = res!(PublicKey::from_bytes(&b[start..end]));
        start = end;
        let sk = res!(SecretKeyCPA::from_bytes(&b[start..]));
        Ok( SecretKeyCCA {
            sk:     sk,
            pk:     pk,
            pk_hash:*hash_pk,
            rand:   *rand,
        })
    }

    pub fn byte_len(&self) -> usize {
        SK_LEN +
        self.pk.byte_len() +
        HASH_BYTES +
        KEY_BYTES
    }
}

#[derive(Clone)]
pub struct CipherText<
    const SBK: usize,
    const PVCB: usize,
> {
    cm: [u8; SBK],
    ct: [u8; PVCB],
}

impl<
    const SBK: usize,
    const PVCB: usize,
>
    CipherText<SBK, PVCB>
{

    fn iter(&self) -> Chain<Iter<u8>, Iter<u8>> {
        self.ct.iter().chain(self.cm.iter())
    }

    pub fn to_vec(&self) -> Vec<u8> { 
        [&self.ct[..], &self.cm[..]].concat()
    }

    pub fn to_bytes<const L: usize>(&self) -> [u8; L] { 
        let mut result = [0u8; L];
        for i in 0..PVCB {
            result[i] = self.ct[i];
        }
        for i in 0..SBK {
            result[PVCB + i] = self.cm[i];
        }
        result
    }

    pub fn from_bytes(b: &[u8]) -> Outcome<Self> {
        let end: usize = PVCB;
        let ct = res!(
            TryInto::<&[u8; PVCB]>::try_into(&b[..end]),
            Conversion, Bytes,
        );
        let cm = res!(
            TryInto::<&[u8; SBK]>::try_into(&b[end..]),
            Conversion, Bytes,
        );
        Ok( CipherText {
            cm: *cm,
            ct: *ct,
        })
    }

    pub fn byte_len(&self) -> usize {
        SBK + PVCB
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::OpenOptions,
        io::{
            BufWriter,
            Write,
        },
    };

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    #[test]
    fn test_polt2bs_lightsaber() {
        // For LightSaber, ET = 3, T = 8
        // Polynomial coefficients, 3 significant bits in every u16
        // msb          lsb
        // ---------------+---------------+---------------+-..
        // ---+---+---+---+---+---+---+---+---+---+---+---+-..
        // 010101010101010101010101010101010101010101010101...
        //              ^^^             ^^^             ^^^
        //  +------------+               |               |
        //  |  +-------------------------+               |
        //  |  |  +--------------------------------------+
        //  v  v  v  v  v  v  v  v  v  v  v  v  v  v  v  v
        // 101101101101101101101101101101101101101101101101
        // ---+---+---+---+---+---+---+---+---+---+---+---+
        // -------+-------+-------+-------+-------+-------+
        // lsb  msb  note: flipped around for diagram 
        // Significant bits of coefficients packed into byte string
        //
        // msb  lsb
        // 01101101
        // 11011011
        // 10110110
        // 01101101
        // 11011011
        // 10110110
        const TEST_N: usize = 2*8;
        const TEST_ET: usize = 3;
        const TEST_SCALE_BYTES_KEM: usize = TEST_ET * TEST_N / 8;
        let poly = [0x5555_u16; TEST_N];
        let saber = LightSaber::default();
        let expected = [
            0b01101101_u8,
            0b11011011,
            0b10110110,
            0b01101101,
            0b11011011,
            0b10110110,
        ];
        let bytes = LightSaber::polt2bs::<TEST_N, TEST_SCALE_BYTES_KEM>(&poly, 0);
        assert_eq!(&bytes, &expected);
    }

    #[test]
    fn test_polt2bs_saber() {
        // For LightSaber, ET = 4, T = 16
        // Polynomial coefficients, 4 significant bits in every u16
        // msb          lsb
        // ---------------+---------------+---------------+-..
        // ---+---+---+---+---+---+---+---+---+---+---+---+-..
        // 010101010101010101010101010101010101010101010101...
        //             ^^^^            ^^^^            ^^^^
        //  +------------+               |               |
        //  |   +------------------------+               |
        //  |   |   +------------------------------------+
        //  v   v   v   v   v   v   v   v   v   v   v   v   v   v   v   v
        // 1010101010101010101010101010101010101010101010101010101010101010
        // ---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
        // -------+-------+-------+-------+-------+-------+-------+-------+
        // lsb  msb  note: flipped around for diagram 
        // Significant bits of coefficients packed into byte string
        //
        // msb  lsb
        // 01010101 x8
        const TEST_N: usize = 2*8;
        const TEST_ET: usize = 4;
        const TEST_SCALE_BYTES_KEM: usize = TEST_ET * TEST_N / 8;
        let poly = [0x5555_u16; TEST_N];
        let saber = LightSaber::default();
        let expected = [0b01010101_u8; 8];
        let bytes = Saber::polt2bs::<TEST_N, TEST_SCALE_BYTES_KEM>(&poly, 0);
        assert_eq!(&bytes, &expected);
    }

    #[test]
    fn test_polt2bs_firesaber() {
        // For FireSaber, ET = 6, T = 64
        // Polynomial coefficients, 6 significant bits in every u16
        // msb          lsb
        // ---------------+---------------+---------------+-..
        // ---+---+---+---+---+---+---+---+---+---+---+---+-..
        // 010101010101010101010101010101010101010101010101...
        //           ^^^^^^          ^^^^^^          ^^^^^^
        //   +----------+               |               |
        //   |     +--------------------+               |
        //   |     |     +------------------------------+
        // vvvvvv  v   vvvvvv  v   ..
        // 101010101010101010101010..
        // ---+---+---+---+---+---+..
        // -------+-------+-------+..
        // lsb  msb  note: flipped around for diagram 
        // Significant bits of coefficients packed into byte string
        //
        // msb  lsb
        // 01010101 x12
        const TEST_N: usize = 2*8;
        const TEST_ET: usize = 6;
        const TEST_SCALE_BYTES_KEM: usize = TEST_ET * TEST_N / 8;
        let poly = [0x5555_u16; TEST_N];
        let saber = LightSaber::default();
        let expected = [0b01010101_u8; 12];
        let bytes = FireSaber::polt2bs::<TEST_N, TEST_SCALE_BYTES_KEM>(&poly, 0);
        assert_eq!(&bytes, &expected);
    }

    #[test]
    fn test_polp2bs() {
        // Polynomial coefficients, 10 significant bits in every u16
        // msb          lsb
        // ---------------+---------------+---------------+-..
        // ---+---+---+---+---+---+---+---+---+---+---+---+-..
        // 011010011010011001101001101001100110100110100110...
        //       ^^^^^^^^^^      ^^^^^^^^^^      ^^^^^^^^^^
        //   +----------+               |               |
        //   |                   +------+               |
        //   |                   |                   +--+
        // vvvvvvvvvv          vvvvvvvvvv          vvvvvvvvvv          vvvvvvvvvv
        // 0110010110011001011001100101100110010110011001011001100101100110010110
        // ---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+..
        // ---1---+---2---+---3---+---4---+---5---+---6---+---7---+---8---+---9---+..
        // lsb  msb  note: flipped around for diagram 
        // Significant bits of coefficients packed into byte string
        //
        //    msb  lsb    lowest common multiple of 8, 10 is 5, so 5 byte cycle
        //  1 10100110
        //  2 10011001
        //  3 01100110
        //  4 10011010
        //  5 01101001
        //  6 ..
        const TEST_N: usize = 2*8;
        const TEST_EP: usize = 10;
        const TEST_POLY_COMPRESSED_BYTES: usize = TEST_EP * TEST_N / 8;
        let poly = [0b0110100110100110_u16; TEST_N];
        let expected_bytes = [
            0b10100110_u8,  //  1
            0b10011001,     //  2
            0b01100110,     //  3
            0b10011010,     //  4
            0b01101001,     //  5
            0b10100110,     //  6
            0b10011001,     //  7
            0b01100110,     //  8
            0b10011010,     //  9
            0b01101001,     // 10
            0b10100110,     // 11
            0b10011001,     // 12
            0b01100110,     // 13
            0b10011010,     // 14
            0b01101001,     // 15
            0b10100110,     // 16
            0b10011001,     // 17
            0b01100110,     // 18
            0b10011010,     // 19
            0b01101001,     // 20
        ];
        let mut bytes = [0; TEST_POLY_COMPRESSED_BYTES];
        Saber::polp2bs::<TEST_N>(&poly, &mut bytes, 0);
        assert_eq!(&bytes, &expected_bytes);
        // byte string -> poly coeffs
        //                     vvvvvv these should be zero, rather than original poly
        let expected_poly = [0b0000000110100110_u16; TEST_N];
        let mut data = [0_u16; TEST_N];
        Saber::bs2polp::<TEST_N>(&bytes, &mut data);
        assert_eq!(&data, &expected_poly);
        // vector of poly coeffs -> byte string
        const TEST_L: usize = 3;
        let poly = [poly; TEST_L];
        let expected: Vec<u8> = vec![expected_bytes.to_vec(); TEST_L]
            .into_iter()
            .flatten()
            .collect();
        let bytes = Saber::polvecp2bs::<
            TEST_N,
            TEST_L,
            TEST_POLY_COMPRESSED_BYTES,
            {TEST_L * TEST_POLY_COMPRESSED_BYTES},
        >(&poly);
        assert_eq!(&bytes.to_vec(), &expected);
    }

    #[test]
    fn test_polq2bs() {
        // Polynomial coefficients, 13 significant bits in every u16
        // msb          lsb
        // ---------------+---------------+---------------+-..
        // ---+---+---+---+---+---+---+---+---+---+---+---+-..
        // 011010011010011001101001101001100110100110100110...
        //    ^^^^^^^^^^^^^   ^^^^^^^^^^^^^   ^^^^^^^^^^^^^
        //   +----------+               |               |
        //   |                         ++               |
        //   |                         |                +--------+
        // vvvvvvvvvvvvv             vvvvvvvvvvvvv             vvvvvvvvvvvvv             vvvvvvvvvvvvv
        // 01100101100100110010110010011001011001001100101100100110010110010011001011001001100101100100110010110010
        // ---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
        // ---1---+---2---+---3---+---4---+---5---+---6---+---7---+---8---+---9---+--10---+--11---+--12---+--13---+
        // lsb  msb  note: flipped around for diagram 
        // Significant bits of coefficients packed into byte string
        //
        //    msb  lsb    lowest common multiple of 8, 13 is 104, so 13 byte cycle
        //  1 10100110
        //  2 11001001
        //  3 00110100
        //  4 10011001
        //  5 00100110
        //  6 11010011
        //  7 01100100
        //  8 10011010
        //  9 01001100
        // 10 10010011
        // 11 01101001
        // 12 00110010
        // 13 01001101
        // 14 ..
        const TEST_N: usize = 2*8;
        const TEST_EQ: usize = 13;
        const TEST_POLY_BYTES: usize = TEST_EQ * TEST_N / 8;
        // poly coeffs -> byte string
        let poly = [0b0110100110100110_u16; TEST_N];
        let expected_bytes = [
            0b10100110_u8,  //  1
            0b11001001,     //  2
            0b00110100,     //  3
            0b10011001,     //  4
            0b00100110,     //  5
            0b11010011,     //  6
            0b01100100,     //  7
            0b10011010,     //  8
            0b01001100,     //  9
            0b10010011,     // 10
            0b01101001,     // 11
            0b00110010,     // 12
            0b01001101,     // 13
            0b10100110,     // 14
            0b11001001,     // 15
            0b00110100,     // 16
            0b10011001,     // 17
            0b00100110,     // 18
            0b11010011,     // 19
            0b01100100,     // 20
            0b10011010,     // 21
            0b01001100,     // 22
            0b10010011,     // 23
            0b01101001,     // 24
            0b00110010,     // 25
            0b01001101,     // 26
        ];
        let mut bytes = [0_u8; TEST_POLY_BYTES];
        Saber::polq2bs::<TEST_N>(&poly, &mut bytes, 0);
        assert_eq!(&bytes, &expected_bytes);
        // byte string -> poly coeffs
        //                     vvv these should be zero, rather than original poly
        let expected_poly = [0b0000100110100110_u16; TEST_N];
        let mut data = [0_u16; TEST_N];
        Saber::bs2polq::<TEST_N>(&bytes, &mut data);
        assert_eq!(&data, &expected_poly);
        // vector of poly coeffs -> byte string
        const TEST_L: usize = 3;
        let poly = [poly; TEST_L];
        let expected: Vec<u8> = vec![expected_bytes.to_vec(); TEST_L]
            .into_iter()
            .flatten()
            .collect();
        let bytes = Saber::polvecq2bs::<
            TEST_N,
            TEST_L,
            TEST_POLY_BYTES,
            {TEST_L * TEST_POLY_BYTES},
        >(&poly);
        assert_eq!(&bytes.to_vec(), &expected);
    }

    #[test]
    fn test_bs2polmsg_polmsg2bs() {
        const TEST_N: usize = 64;
        const TEST_KEY_BYTES: usize = 8;
        let msg_in = [
            0b01001101_u8,
            0b01010001,
            0b11110110,
            0b00001101,
            0b10000110,
            0b01010111,
            0b00100000,
            0b00111010,
        ];
        let poly = Saber::bs2polmsg::<TEST_N, TEST_KEY_BYTES>(&msg_in);
        let msg_out = Saber::polmsg2bs::<TEST_N, TEST_KEY_BYTES>(&poly);
        assert_eq!(&msg_in, &msg_out);
    }

    #[derive(Default)]
    struct SaberC {
        client_secret: [u8; KEY_BYTES],
        server_secret: [u8; KEY_BYTES],
        entropy_input: Vec<u8>,
        pk: Vec<u8>,
        sk: Vec<u8>,
        ct: Vec<u8>,
    }

    fn call_c<
        const PK: usize,
        const SK: usize,
        const CT: usize,
    >(
        mut c: &mut SaberC
    )
    {
        let mut pk = [0_u8; PK];
        let mut sk = [0_u8; SK];
        let mut ct = [0_u8; CT];
        unsafe {
            randombytes_init(
                c.entropy_input.as_mut_ptr(), 
                std::ptr::null_mut(),
                256,
            );
            crypto_kem_keypair(
                pk.as_mut_ptr(),
                sk.as_mut_ptr(),
            );
            crypto_kem_enc(
                ct.as_mut_ptr(),
                c.client_secret.as_mut_ptr(),
                pk.as_ptr(),
            );
            crypto_kem_dec(
                c.server_secret.as_mut_ptr(),
                ct.as_ptr(),
                sk.as_ptr(),
            );
        }
        c.pk = pk.to_vec();
        c.sk = sk.to_vec();
        c.ct = ct.to_vec();
    }

    #[test]
    fn test_validate_kem() -> Outcome<()> {

        let which = env!("SABER_SCHEME");

        let mut c = SaberC::default();

        for i in 0..48_usize {
            c.entropy_input.push(i as u8);
        }

        match which {
            "LIGHTSABER" => {
                call_c::<
                    {LightSaber::PUBLIC_KEY_BYTES},
                    {LightSaber::SECRET_KEY_BYTES},
                    {LightSaber::CIPHERTEXT_BYTES},
                >(&mut c);
            },
            "SABER" => {
                call_c::<
                    {Saber::PUBLIC_KEY_BYTES},
                    {Saber::SECRET_KEY_BYTES},
                    {Saber::CIPHERTEXT_BYTES},
                >(&mut c);
            },
            "FIRESABER" => {
                call_c::<
                    {FireSaber::PUBLIC_KEY_BYTES},
                    {FireSaber::SECRET_KEY_BYTES},
                    {FireSaber::CIPHERTEXT_BYTES},
                >(&mut c);
            },
            _ => unimplemented!(),
        }

        #[cfg(SABER_SCHEME = "LIGHTSABER")]
        let scheme = LightSaber;
        #[cfg(SABER_SCHEME = "LIGHTSABER")]
        const CT_LEN: usize = LightSaber::CIPHERTEXT_BYTES;

        #[cfg(SABER_SCHEME = "SABER")]
        let scheme = Saber;
        #[cfg(SABER_SCHEME = "SABER")]
        const CT_LEN: usize = Saber::CIPHERTEXT_BYTES;

        #[cfg(SABER_SCHEME = "FIRESABER")]
        let scheme = FireSaber;
        #[cfg(SABER_SCHEME = "FIRESABER")]
        const CT_LEN: usize = FireSaber::CIPHERTEXT_BYTES;

        msg!("{} parameters:", scheme);
        msg!("PUBLIC_KEY_BYTES: {}", scheme.pk_len());
        msg!("SECRET_KEY_BYTES: {}", scheme.sk_len());
        msg!("CIPHERTEXT_BYTES: {}", scheme.ct_len());

        let seed_a = [
            0x06, 0x15, 0x50, 0x23, 0x4d, 0x15, 0x8c, 0x5e,
            0xc9, 0x55, 0x95, 0xfe, 0x04, 0xef, 0x7a, 0x25, 
            0x76, 0x7f, 0x2e, 0x24, 0xcc, 0x2b, 0xc4, 0x79,
            0xd0, 0x9d, 0x86, 0xdc, 0x9a, 0xbc, 0xfd, 0xe7, 
        ];
        let seed_s = [
            0x1a, 0x9f, 0xbc, 0xbc, 0x8d, 0xa3, 0x6d, 0xff,
            0x2a, 0xbe, 0x20, 0x32, 0x96, 0x17, 0x0f, 0xdb,
            0x97, 0xc3, 0x29, 0x7f, 0x67, 0xfc, 0xb6, 0x79,
            0xac, 0x71, 0x9c, 0x9f, 0xd0, 0x02, 0x53, 0xb0,
        ];
        let rand = [
            0xb2, 0xf0, 0x04, 0xf5, 0x43, 0x5f, 0x10, 0xc4,
            0xcd, 0x45, 0x11, 0x48, 0x44, 0x7a, 0xfd, 0x9b,
            0x99, 0xb2, 0x09, 0x77, 0x0d, 0xe0, 0xd0, 0x3a,
            0xcd, 0xb7, 0xbc, 0x6b, 0xe5, 0x71, 0x68, 0x8c,
        ];
        let mut enc_seed = [
            0x78, 0x97, 0x71, 0x80, 0x42, 0xad, 0x01, 0x0b,
            0xc9, 0x8b, 0xe9, 0x5d, 0x13, 0xdd, 0xde, 0xf0,
            0x65, 0x33, 0xab, 0x95, 0x42, 0x6f, 0xaf, 0xc7,
            0x49, 0x76, 0xcd, 0x99, 0xad, 0xb7, 0x45, 0x62,
        ];

        let (pk, sk) = scheme.kem_keygen_test(seed_a, seed_s, rand);

        let file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(true)
            .append(false)
            .open(format!("validation_output_for_{}.txt", which))
            .unwrap();
        let mut fb = BufWriter::new(file);

        writeln!(&mut fb, "Rust secret key: length {}", sk.byte_len());
        for line in dump!(" {:02x}", sk.iter(), 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "C secret key: length {}", c.sk.len());
        for line in dump!(" {:02x}", &c.sk, 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "Rust public key: length {}", pk.byte_len());
        for line in dump!(" {:02x}", pk.iter(), 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "C public key: length {}", c.pk.len());
        for line in dump!(" {:02x}", &c.pk, 16) {
            writeln!(&mut fb, "{}", line);
        }

        assert_eq!(pk.byte_len(), c.pk.len());
        assert_eq!(sk.byte_len(), c.sk.len());

        for (i, pki) in pk.iter().enumerate() {
            assert_eq!(pki, &c.pk[i], "failed at i = {}", i);
        }

        for (i, ski) in sk.iter().enumerate() {
            assert_eq!(ski, &c.sk[i], "failed at i = {}", i);
        }

        let (client_secret, ciphertext) = scheme.kem_encap_test(&pk, enc_seed);
        writeln!(&mut fb, "Rust client secret: length {}", client_secret.len());
        for line in dump!(" {:02x}", client_secret.iter(), 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "C client secret: length {}", c.client_secret.len());
        for line in dump!(" {:02x}", &c.client_secret, 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "Rust ciphertext: length {}", ciphertext.byte_len());
        for line in dump!(" {:02x}", ciphertext.iter(), 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "C ciphertext: length {}", c.ct.len());
        for line in dump!(" {:02x}", &c.ct, 16) {
            writeln!(&mut fb, "{}", line);
        }

        let server_secret = res!(scheme.kem_decap(&ciphertext.to_bytes::<{CT_LEN}>(), &sk));
        writeln!(&mut fb, "Rust server secret: length {}", server_secret.len());
        for line in dump!(" {:02x}", server_secret.iter(), 16) {
            writeln!(&mut fb, "{}", line);
        }
        writeln!(&mut fb, "C server secret: length {}", c.server_secret.len());
        for line in dump!(" {:02x}", &c.server_secret, 16) {
            writeln!(&mut fb, "{}", line);
        }

        for i in 0..client_secret.len() {
            assert_eq!(client_secret[i], server_secret[i], "failed at i = {}", i);
        }
        
        Ok(())
    }

    #[test]
    fn test_lightsaber_kem() -> Outcome<()> {

        let scheme = LightSaber;

        msg!("{} parameters:", scheme);
        msg!("PUBLIC_KEY_BYTES: {}", scheme.pk_len());
        msg!("SECRET_KEY_BYTES: {}", scheme.sk_len());
        msg!("CIPHERTEXT_BYTES: {}", scheme.ct_len());

        for _ in 0..1000 {
            let (pk, sk) = scheme.kem_keygen();
            let (client_secret, ciphertext) = scheme.kem_encap(&pk);
            let server_secret = res!(scheme.kem_decap(
                &ciphertext.to_bytes::<{LightSaber::CIPHERTEXT_BYTES}>(),
                &sk,
            ));
            for i in 0..client_secret.len() {
                assert_eq!(client_secret[i], server_secret[i], "failed at i = {}", i);
            }
        }
        
        Ok(())
    }

    #[test]
    fn test_saber_kem() -> Outcome<()> {

        let scheme = Saber;

        msg!("{} parameters:", scheme);
        msg!("PUBLIC_KEY_BYTES: {}", scheme.pk_len());
        msg!("SECRET_KEY_BYTES: {}", scheme.sk_len());
        msg!("CIPHERTEXT_BYTES: {}", scheme.ct_len());

        for _ in 0..1000 {
            let (pk, sk) = scheme.kem_keygen();
            let (client_secret, ciphertext) = scheme.kem_encap(&pk);
            let server_secret = res!(scheme.kem_decap(
                &ciphertext.to_bytes::<{Saber::CIPHERTEXT_BYTES}>(),
                &sk,
            ));
            for i in 0..client_secret.len() {
                assert_eq!(client_secret[i], server_secret[i], "failed at i = {}", i);
            }
        }
        
        Ok(())
    }

    #[test]
    fn test_firesaber_kem() -> Outcome<()> {

        let scheme = FireSaber;

        msg!("{} parameters:", scheme);
        msg!("PUBLIC_KEY_BYTES: {}", scheme.pk_len());
        msg!("SECRET_KEY_BYTES: {}", scheme.sk_len());
        msg!("CIPHERTEXT_BYTES: {}", scheme.ct_len());

        for _ in 0..1000 {
            let (pk, sk) = scheme.kem_keygen();
            let (client_secret, ciphertext) = scheme.kem_encap(&pk);
            let server_secret = res!(scheme.kem_decap(
                &ciphertext.to_bytes::<{FireSaber::CIPHERTEXT_BYTES}>(),
                &sk,
            ));
            for i in 0..client_secret.len() {
                assert_eq!(client_secret[i], server_secret[i], "failed at i = {}", i);
            }
        }
        
        Ok(())
    }
}
