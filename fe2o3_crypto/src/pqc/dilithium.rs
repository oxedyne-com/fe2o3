//! # Crystals-Dilithium
//! This is a pure rust implementation by https://github.com/quininer of the NIST post quantum
//! cryptography digital signature finalist.
//!
//! I have so far collected their implementation files into a single file and added some rough
//! speed tests to compare with Ed25519 signing, along with one or two top level api tweaks.
//! Although performance optimisations can be expected, the disparity gave rise to the notion of
//! using a dual signature scheme in the Ozone database, whereby hashes of items are signed using
//! the fast Ed25519 algorithm, and a hash of these hashes is signed using Dilithium.
//!

mod ntt {

    use super::params::*;
    use super::reduce::montgomery_reduce;

    use itertools::Itertools;

    pub fn ntt(p: &mut [u32; N]) {
        let mut k = 1;
        for len in (0..8).map(|level| 1 << level).rev() {
            for start in Itertools::step(0..N, 2 * len) {
                let zeta = u64::from(ZETAS[k]);
                k += 1;
    
                for j in start..(start + len) {
                    let t = montgomery_reduce(zeta * u64::from(p[j + len]));
                    p[j + len] = p[j] + 2 * Q - t;
                    p[j] += t;
                }
            }
        }
    }
    
    pub fn invntt_frominvmont(p: &mut [u32; N]) {
        const F: u64 = ((MONT * MONT % (Q as u64))  * (Q as u64 - 1) % (Q as u64)) * ((Q as u64 - 1) >> 8) % (Q as u64);
    
        let mut k = 1;
        for len in (0..8).map(|level| 1 << level) {
            for start in Itertools::step(0..N, 2 * len) {
                let zeta = u64::from(ZETAS_INV[k]);
                k += 1;
    
                for j in start..(start + len) {
                    let t = p[j];
                    p[j] += p[j + len];
                    p[j + len] = t + 256 * Q - p[j + len];
                    p[j + len] = montgomery_reduce(zeta * u64::from(p[j + len]));
                }
            }
        }
    
        for j in 0..N {
            p[j] = montgomery_reduce(F * u64::from(p[j]));
        }
    }
}

//////////////////////////////////////////////////////////////////////////// Original file packing.rs

mod packing {

    use super::params::*;
    use super::poly::{
        self,
        Poly,
    };
    use super::polyvec::{
        PolyVecL,
        PolyVecK,
    };
    
    use arrayref::{
        array_ref,
        array_refs,
        array_mut_ref,
        mut_array_refs,
    };

    pub mod pk {
        use super::*;
    
        pub fn pack(pk: &mut [u8; PK_SIZE_PACKED], rho: &[u8; SEEDBYTES], t1: &PolyVecK) {
            let (rho_bytes, t1s_bytes) = mut_array_refs!(pk, SEEDBYTES, POLT1_SIZE_PACKED * K);
    
            rho_bytes.clone_from(rho);
            for i in 0..K {
                let t1_bytes = array_mut_ref!(t1s_bytes, i * POLT1_SIZE_PACKED, POLT1_SIZE_PACKED);
                poly::t1_pack(t1_bytes, &t1[i]);
            }
        }
    
        pub fn unpack(pk: &[u8; PK_SIZE_PACKED], rho: &mut [u8; SEEDBYTES], t1: &mut PolyVecK) {
            let (rho_bytes, t1s_bytes) = array_refs!(pk, SEEDBYTES, POLT1_SIZE_PACKED * K);
    
            rho.clone_from(rho_bytes);
            for i in 0..K {
                let t1_bytes = array_ref!(t1s_bytes, i * POLT1_SIZE_PACKED, POLT1_SIZE_PACKED);
                poly::t1_unpack(&mut t1[i], t1_bytes);
            }
        }
    }
    
    pub mod sk {
        use super::*;
    
        pub fn pack(
            sk: &mut [u8; SK_SIZE_PACKED],
            rho: &[u8; SEEDBYTES],
            key: &[u8; SEEDBYTES],
            tr: &[u8; CRHBYTES],
            s1: &PolyVecL,
            s2: &PolyVecK,
            t0: &PolyVecK
        ) {
            let (rho_bytes, key_bytes, tr_bytes, s1s_bytes, s2s_bytes, t0s_bytes) =
                mut_array_refs!(
                    sk,
                    SEEDBYTES, SEEDBYTES, CRHBYTES,
                    POLETA_SIZE_PACKED * L,
                    POLETA_SIZE_PACKED * K,
                    POLT0_SIZE_PACKED * K
                );
    
            rho_bytes.clone_from(rho);
            key_bytes.clone_from(key);
            tr_bytes.clone_from(tr);
    
            for i in 0..L {
                let s1_bytes = array_mut_ref!(s1s_bytes, i * POLETA_SIZE_PACKED, POLETA_SIZE_PACKED);
                poly::eta_pack(s1_bytes, &s1[i]);
            }
            for i in 0..K {
                let s2_bytes = array_mut_ref!(s2s_bytes, i * POLETA_SIZE_PACKED, POLETA_SIZE_PACKED);
                poly::eta_pack(s2_bytes, &s2[i]);
            }
            for i in 0..K {
                let t0_bytes = array_mut_ref!(t0s_bytes, i * POLT0_SIZE_PACKED, POLT0_SIZE_PACKED);
                poly::t0_pack(t0_bytes, &t0[i]);
            }
        }
    
        pub fn unpack(
            sk: &[u8; SK_SIZE_PACKED],
            rho: &mut [u8; SEEDBYTES],
            key: &mut [u8; SEEDBYTES],
            tr: &mut [u8; CRHBYTES],
            s1: &mut PolyVecL,
            s2: &mut PolyVecK,
            t0: &mut PolyVecK
       ) {
            let (rho_bytes, key_bytes, tr_bytes, s1s_bytes, s2s_bytes, t0s_bytes) =
                array_refs!(
                    sk,
                    SEEDBYTES, SEEDBYTES, CRHBYTES,
                    POLETA_SIZE_PACKED * L,
                    POLETA_SIZE_PACKED * K,
                    POLT0_SIZE_PACKED * K
                );
    
            rho.clone_from(rho_bytes);
            key.clone_from(key_bytes);
            tr.clone_from(tr_bytes);
    
            for i in 0..L {
                let s1_bytes = array_ref!(s1s_bytes, i * POLETA_SIZE_PACKED, POLETA_SIZE_PACKED);
                poly::eta_unpack(&mut s1[i], s1_bytes);
            }
            for i in 0..K {
                let s2_bytes = array_ref!(s2s_bytes, i * POLETA_SIZE_PACKED, POLETA_SIZE_PACKED);
                poly::eta_unpack(&mut s2[i], s2_bytes);
            }
            for i in 0..K {
                let t0_bytes = array_ref!(t0s_bytes, i * POLT0_SIZE_PACKED, POLT0_SIZE_PACKED);
                poly::t0_unpack(&mut t0[i], t0_bytes);
            }
        }
    }
    
    pub mod sign {
        use super::*;
    
        pub fn pack(sign: &mut [u8; SIG_SIZE_PACKED], z: &PolyVecL, h: &PolyVecK,c: &Poly) {
            let (zs_bytes, h_bytes, c_bytes) =
                mut_array_refs!(
                    sign,
                    POLZ_SIZE_PACKED * L,
                    OMEGA + K,
                    N / 8 + 8
                );
    
            for i in 0..L {
                let z_bytes = array_mut_ref!(zs_bytes, i * POLZ_SIZE_PACKED, POLZ_SIZE_PACKED);
                poly::z_pack(z_bytes, &z[i]);
            }
    
            let mut k = 0;
            for i in 0..K {
                for j in 0..N {
                    if h[i][j] != 0 {
                        h_bytes[k] = j as u8;
                        k += 1;
                    }
                }
                h_bytes[OMEGA + i] = k as u8;
            }
    
            let mut signs: u64 = 0;
            let mut mask = 1;
            for i in 0..(N / 8) {
                for j in 0..8 {
                    if c[8 * i + j] != 0 {
                        c_bytes[i] |= 1 << j;
                        if c[8 * i + j] == Q - 1 {
                            signs |= mask;
                        }
                        mask <<= 1;
                    }
                }
            }
            for i in 0..8 {
                c_bytes[N / 8..][i] = (signs >> (8 * i)) as u8;
            }
        }
    
        pub fn unpack(sign: &[u8; SIG_SIZE_PACKED], z: &mut PolyVecL, h: &mut PolyVecK, c: &mut Poly) -> bool {
            let (zs_bytes, h_bytes, c_bytes) =
                array_refs!(
                    sign,
                    POLZ_SIZE_PACKED * L,
                    OMEGA + K,
                    N / 8 + 8
                );
    
            for i in 0..L {
                let z_bytes = array_ref!(zs_bytes, i * POLZ_SIZE_PACKED, POLZ_SIZE_PACKED);
                poly::z_unpack(&mut z[i], z_bytes);
            }
    
            // Decode h
            let mut k = 0;
            for i in 0..K {
                if (h_bytes[OMEGA + i] as usize) < k || (h_bytes[OMEGA + i] as usize) > OMEGA {
                    return false;
                }
    
                for j in k..(h_bytes[OMEGA + i] as usize) {
                    // Coefficients are ordered for strong unforgeability
                    if j > k && h_bytes[j] <= h_bytes[j - 1] {
                        return false;
                    }
    
                    h[i][h_bytes[j] as usize] = 1;
                }
                k = h_bytes[OMEGA + i] as usize;
            }
    
            // Extra indices are zero for strong unforgeability
            if h_bytes[k..OMEGA].iter().any(|&v| v != 0) {
                return false;
            }
    
            let signs = (0..8)
                .map(|i| u64::from(c_bytes[N / 8 + i]) << (8 * i))
                .fold(0, |sum, next| sum | next);
    
            // Extra sign bits are zero for strong unforgeability
            if signs >> 60 != 0 {
                return false;
            }
    
            let mut mask = 1;
            for i in 0..(N / 8) {
                for j in 0..8 {
                    if (c_bytes[i] >> j) & 0x01 != 0 {
                        c[8 * i + j] =
                            if (signs & mask) != 0 { Q - 1 }
                            else { 1 };
                        mask <<= 1;
                    }
                }
            }
    
            true
        }
    }
}

///////////////////////////////////////////////////////////////////////// Original file params.rs

pub mod params {
    pub const SEEDBYTES    : usize = 32;
    pub const CRHBYTES     : usize = 48;
    pub const N            : usize = 256;
    pub const Q            : u32   = 8380417;
    pub const QBITS        : usize = 23;
    pub const ROOT_OF_UNITY: usize = 1753;
    pub const D            : usize = 14;
    pub const GAMMA1       : u32   = (Q - 1) / 16;
    pub const GAMMA2       : u32   = GAMMA1 / 2;
    pub const ALPHA        : u32   = 2 * GAMMA2;
    
    #[cfg(feature = "mode0")]
    mod mode {
        pub const K       : usize = 3;
        pub const L       : usize = 2;
        pub const ETA     : u32   = 7;
        pub const SETABITS: usize = 4;
        pub const BETA    : u32   = 375;
        pub const OMEGA   : usize = 64;
    }
    
    #[cfg(feature = "mode1")]
    mod mode {
        pub const K       : usize = 4;
        pub const L       : usize = 3;
        pub const ETA     : u32   = 6;
        pub const SETABITS: usize = 4;
        pub const BETA    : u32   = 325;
        pub const OMEGA   : usize = 80;
    }
    
    #[cfg(feature = "mode2")]
    mod mode {
        pub const K       : usize = 5;
        pub const L       : usize = 4;
        pub const ETA     : u32   = 5;
        pub const SETABITS: usize = 4;
        pub const BETA    : u32   = 275;
        pub const OMEGA   : usize = 96;
    }
    
    #[cfg(feature = "mode3")]
    mod mode {
        pub const K       : usize = 6;
        pub const L       : usize = 5;
        pub const ETA     : u32   = 3;
        pub const SETABITS: usize = 3;
        pub const BETA    : u32   = 175;
        pub const OMEGA   : usize = 120;
    }
    
    pub use self::mode::*;
    
    pub const POL_SIZE_PACKED   : usize = (N * QBITS) / 8;
    pub const POLT1_SIZE_PACKED : usize = (N * (QBITS - D)) / 8;
    pub const POLT0_SIZE_PACKED : usize = (N * D) / 8;
    pub const POLETA_SIZE_PACKED: usize = (N * SETABITS) / 8;
    pub const POLZ_SIZE_PACKED  : usize = (N * (QBITS - 3)) / 8;
    pub const POLW1_SIZE_PACKED : usize = (N * 4) / 8;
    
    pub const POLVECK_SIZE_PACKED: usize = K * POL_SIZE_PACKED;
    pub const POLVECL_SIZE_PACKED: usize = L * POL_SIZE_PACKED;
    pub const PK_SIZE_PACKED     : usize = SEEDBYTES + K * POLT1_SIZE_PACKED;
    pub const SK_SIZE_PACKED     : usize = 2 * SEEDBYTES + (L + K) * POLETA_SIZE_PACKED + CRHBYTES + K * POLT0_SIZE_PACKED;
    pub const SIG_SIZE_PACKED    : usize = L * POLZ_SIZE_PACKED + (OMEGA + K) + (N / 8 + 8);
    
    pub const PUBLICKEYBYTES: usize = PK_SIZE_PACKED;
    pub const SECRETKEYBYTES: usize = SK_SIZE_PACKED;
    pub const BYTES         : usize = SIG_SIZE_PACKED;
    
    pub const MONT: u64   = 4193792;
    pub const QINV: usize = 4236238847;
    
    pub const ZETAS: [u32; N] = [0, 25847, 5771523, 7861508, 237124, 7602457, 7504169, 466468, 1826347, 2353451, 8021166, 6288512, 3119733, 5495562, 3111497, 2680103, 2725464, 1024112, 7300517, 3585928, 7830929, 7260833, 2619752, 6271868, 6262231, 4520680, 6980856, 5102745, 1757237, 8360995, 4010497, 280005, 2706023, 95776, 3077325, 3530437, 6718724, 4788269, 5842901, 3915439, 4519302, 5336701, 3574422, 5512770, 3539968, 8079950, 2348700, 7841118, 6681150, 6736599, 3505694, 4558682, 3507263, 6239768, 6779997, 3699596, 811944, 531354, 954230, 3881043, 3900724, 5823537, 2071892, 5582638, 4450022, 6851714, 4702672, 5339162, 6927966, 3475950, 2176455, 6795196, 7122806, 1939314, 4296819, 7380215, 5190273, 5223087, 4747489, 126922, 3412210, 7396998, 2147896, 2715295, 5412772, 4686924, 7969390, 5903370, 7709315, 7151892, 8357436, 7072248, 7998430, 1349076, 1852771, 6949987, 5037034, 264944, 508951, 3097992, 44288, 7280319, 904516, 3958618, 4656075, 8371839, 1653064, 5130689, 2389356, 8169440, 759969, 7063561, 189548, 4827145, 3159746, 6529015, 5971092, 8202977, 1315589, 1341330, 1285669, 6795489, 7567685, 6940675, 5361315, 4499357, 4751448, 3839961, 2091667, 3407706, 2316500, 3817976, 5037939, 2244091, 5933984, 4817955, 266997, 2434439, 7144689, 3513181, 4860065, 4621053, 7183191, 5187039, 900702, 1859098, 909542, 819034, 495491, 6767243, 8337157, 7857917, 7725090, 5257975, 2031748, 3207046, 4823422, 7855319, 7611795, 4784579, 342297, 286988, 5942594, 4108315, 3437287, 5038140, 1735879, 203044, 2842341, 2691481, 5790267, 1265009, 4055324, 1247620, 2486353, 1595974, 4613401, 1250494, 2635921, 4832145, 5386378, 1869119, 1903435, 7329447, 7047359, 1237275, 5062207, 6950192, 7929317, 1312455, 3306115, 6417775, 7100756, 1917081, 5834105, 7005614, 1500165, 777191, 2235880, 3406031, 7838005, 5548557, 6709241, 6533464, 5796124, 4656147, 594136, 4603424, 6366809, 2432395, 2454455, 8215696, 1957272, 3369112, 185531, 7173032, 5196991, 162844, 1616392, 3014001, 810149, 1652634, 4686184, 6581310, 5341501, 3523897, 3866901, 269760, 2213111, 7404533, 1717735, 472078, 7953734, 1723600, 6577327, 1910376, 6712985, 7276084, 8119771, 4546524, 5441381, 6144432, 7959518, 6094090, 183443, 7403526, 1612842, 4834730, 7826001, 3919660, 8332111, 7018208, 3937738, 1400424, 7534263, 1976782];
    pub const ZETAS_INV: [u32; N] = [0, 6403635, 846154, 6979993, 4442679, 1362209, 48306, 4460757, 554416, 3545687, 6767575, 976891, 8196974, 2286327, 420899, 2235985, 2939036, 3833893, 260646, 1104333, 1667432, 6470041, 1803090, 6656817, 426683, 7908339, 6662682, 975884, 6167306, 8110657, 4513516, 4856520, 3038916, 1799107, 3694233, 6727783, 7570268, 5366416, 6764025, 8217573, 3183426, 1207385, 8194886, 5011305, 6423145, 164721, 5925962, 5948022, 2013608, 3776993, 7786281, 3724270, 2584293, 1846953, 1671176, 2831860, 542412, 4974386, 6144537, 7603226, 6880252, 1374803, 2546312, 6463336, 1279661, 1962642, 5074302, 7067962, 451100, 1430225, 3318210, 7143142, 1333058, 1050970, 6476982, 6511298, 2994039, 3548272, 5744496, 7129923, 3767016, 6784443, 5894064, 7132797, 4325093, 7115408, 2590150, 5688936, 5538076, 8177373, 6644538, 3342277, 4943130, 4272102, 2437823, 8093429, 8038120, 3595838, 768622, 525098, 3556995, 5173371, 6348669, 3122442, 655327, 522500, 43260, 1613174, 7884926, 7561383, 7470875, 6521319, 7479715, 3193378, 1197226, 3759364, 3520352, 4867236, 1235728, 5945978, 8113420, 3562462, 2446433, 6136326, 3342478, 4562441, 6063917, 4972711, 6288750, 4540456, 3628969, 3881060, 3019102, 1439742, 812732, 1584928, 7094748, 7039087, 7064828, 177440, 2409325, 1851402, 5220671, 3553272, 8190869, 1316856, 7620448, 210977, 5991061, 3249728, 6727353, 8578, 3724342, 4421799, 7475901, 1100098, 8336129, 5282425, 7871466, 8115473, 3343383, 1430430, 6527646, 7031341, 381987, 1308169, 22981, 1228525, 671102, 2477047, 411027, 3693493, 2967645, 5665122, 6232521, 983419, 4968207, 8253495, 3632928, 3157330, 3190144, 1000202, 4083598, 6441103, 1257611, 1585221, 6203962, 4904467, 1452451, 3041255, 3677745, 1528703, 3930395, 2797779, 6308525, 2556880, 4479693, 4499374, 7426187, 7849063, 7568473, 4680821, 1600420, 2140649, 4873154, 3821735, 4874723, 1643818, 1699267, 539299, 6031717, 300467, 4840449, 2867647, 4805995, 3043716, 3861115, 4464978, 2537516, 3592148, 1661693, 4849980, 5303092, 8284641, 5674394, 8100412, 4369920, 19422, 6623180, 3277672, 1399561, 3859737, 2118186, 2108549, 5760665, 1119584, 549488, 4794489, 1079900, 7356305, 5654953, 5700314, 5268920, 2884855, 5260684, 2091905, 359251, 6026966, 6554070, 7913949, 876248, 777960, 8143293, 518909, 2608894, 8354570];

}

//////////////////////////////////////////////////////////////////////////////////// Original file poly.rs

mod poly {

    use super::params::*;
    use super::reduce::{
        reduce32,
        montgomery_reduce,
        freeze as xfreeze,
        csubq as xcsubq,
    };
    use super::rounding;
    pub use super::ntt::{
        ntt,
        invntt_frominvmont as invntt_montgomery,
    };

    use byteorder::{ ByteOrder, LittleEndian };

    pub type Poly = [u32; N];
    
    pub fn reduce(a: &mut Poly) {
        for i in 0..N {
            a[i] = reduce32(a[i]);
        }
    }
    
    pub fn csubq(a: &mut Poly) {
        for i in 0..N {
            a[i] = xcsubq(a[i]);
        }
    }
    
    pub fn freeze(a: &mut Poly) {
        for i in 0..N {
            a[i] = xfreeze(a[i]);
        }
    }
    
    pub fn add(c: &mut Poly, a: &Poly, b: &Poly) {
        for i in 0..N {
            c[i] = a[i] + b[i];
        }
    }
    
    pub fn add_assign(c: &mut Poly, a: &Poly) {
        for i in 0..N {
            c[i] += a[i];
        }
    }
    
    pub fn sub(c: &mut Poly, a: &Poly, b: &Poly) {
        for i in 0..N {
            c[i] = a[i] + 2 * Q -  b[i];
        }
    }
    
    pub fn shift_left(a: &mut Poly, k: u32) {
        for i in 0..N {
            a[i] <<= k;
        }
    }
    
    pub fn pointwise_invmontgomery(c: &mut Poly, a: &Poly, b: &Poly) {
        for i in 0..N {
            c[i] = montgomery_reduce(u64::from(a[i]) * u64::from(b[i]));
        }
    }
    
    pub fn power2round(a: &Poly, a0: &mut Poly, a1: &mut Poly) {
        for i in 0..N {
            let (x, y) = rounding::power2round(a[i]);
            a0[i] = x;
            a1[i] = y;
        }
    }
    
    pub fn decompose(a: &Poly, a0: &mut Poly, a1: &mut Poly) {
        for i in 0..N {
            let (x, y) = rounding::decompose(a[i]);
            a0[i] = x;
            a1[i] = y;
        }
    }
    
    pub fn make_hint(a: &Poly, b: &Poly, h: &mut Poly) -> usize {
        let mut s = 0;
    
        for i in 0..N {
            h[i] = rounding::make_hint(a[i], b[i]);
            s += h[i] as usize;
        }
    
        s
    }
    
    pub fn use_hint(a: &mut Poly, b: &Poly, h: &Poly) {
        for i in 0..N {
            a[i] = rounding::use_hint(b[i], h[i]);
        }
    }
    
    pub fn chknorm(a: &Poly, b: u32) -> bool {
        a.iter()
            .map(|&a|{
                let mut t = ((Q - 1) / 2).wrapping_sub(a) as i32;
                t ^= t >> 31;
                ((Q - 1) / 2) as i32 - t
            })
            .any(|t| t as u32 >= b)
    }
    
    pub fn uniform(a: &mut Poly, buf: &[u8]) {
        let mut ctr = 0;
        let mut pos = 0;
    
        while ctr < N {
            let val = LittleEndian::read_u24(&buf[pos..]) & 0x7f_ffff;
            pos += 3;
    
            if val < Q {
                a[ctr] = val;
                ctr += 1;
            }
        }
    }
    
    pub fn uniform_eta(a: &mut Poly, seed: &[u8; SEEDBYTES], nonce: u8) {
        use digest::{ Input, ExtendableOutput, XofReader };
        use sha3::Shake256;
    
        const SHAKE256_RATE: usize = 136;
    
        fn rej_eta(a: &mut [u32], buf: &[u8]) -> usize {
            let mut ctr = 0;
            let mut pos = 0;
            let len = a.len();
    
            while ctr < len && pos < buf.len() {
                let (t0, t1) =
                    if ETA <= 3 { (u32::from(buf[pos] & 0x07), u32::from(buf[pos] >> 5)) }
                    else { (u32::from(buf[pos] & 0x0f), u32::from(buf[pos] >> 4)) };
                pos += 1;
    
                if t0 <= 2 * ETA {
                    a[ctr] = Q + ETA - t0;
                    ctr += 1;
                }
                if t1 <= 2 * ETA && ctr < len {
                    a[ctr] = Q + ETA - t1;
                    ctr += 1;
                }
    
                if pos >= buf.len() {
                    break
                }
            }
    
            ctr
        }
    
        let mut outbuf = [0; 2 * SHAKE256_RATE];
        let mut hasher = Shake256::default();
        hasher.process(seed);
        hasher.process(&[nonce]);
    
        let mut xof = hasher.xof_result();
        xof.read(&mut outbuf);
    
        let ctr = rej_eta(a, &outbuf);
        if ctr < N {
            xof.read(&mut outbuf[..SHAKE256_RATE]);
            rej_eta(&mut a[ctr..], &outbuf[..SHAKE256_RATE]);
        }
    }
    
    pub fn uniform_gamma1m1(a: &mut Poly, seed: &[u8; SEEDBYTES], mu: &[u8; CRHBYTES], nonce: u16) {
        use digest::{ Input, ExtendableOutput, XofReader };
        use sha3::Shake256;
    
        const SHAKE256_RATE: usize = 136;
    
        fn rej_gemma1m1(a: &mut [u32], buf: &[u8]) -> usize {
            let mut ctr = 0;
            let mut pos = 0;
    
            while ctr < a.len() && pos + 5 <= buf.len() {
                let mut t0 = u32::from(buf[pos]);
                t0 |= u32::from(buf[pos + 1]) << 8;
                t0 |= u32::from(buf[pos + 2]) << 16;
                t0 &= 0xfffff;
    
                let mut t1 = u32::from(buf[pos + 2]) >> 4;
                t1 |= u32::from(buf[pos + 3]) << 4;
                t1 |= u32::from(buf[pos + 4]) << 12;
    
                pos += 5;
    
                if t0 <= 2 * GAMMA1 - 2 {
                    a[ctr] = Q + GAMMA1 - 1 - t0;
                    ctr += 1;
                }
                if t1 <= 2 * GAMMA1 - 2 && ctr < a.len() {
                    a[ctr] = Q + GAMMA1 - 1 - t1;
                    ctr += 1;
                }
    
                if pos > buf.len() - 5 {
                    break
                }
            }
    
            ctr
        }
    
        let mut outbuf = [0; 5 * SHAKE256_RATE];
        let mut nonce_bytes = [0; 2];
        LittleEndian::write_u16(&mut nonce_bytes, nonce);
    
        let mut hasher = Shake256::default();
        hasher.process(seed);
        hasher.process(mu);
        hasher.process(&nonce_bytes);
    
        let mut xof = hasher.xof_result();
        xof.read(&mut outbuf);
    
        let ctr = rej_gemma1m1(a, &outbuf);
        if ctr < N {
            xof.read(&mut outbuf[..SHAKE256_RATE]);
            rej_gemma1m1(&mut a[ctr..], &outbuf[..SHAKE256_RATE]);
        }
    }
    
    #[inline]
    pub fn eta_pack(r: &mut [u8; POLETA_SIZE_PACKED], a: &Poly) {
        if ETA <= 3 {
            let mut t = [0; 8];
            for i in 0..(N / 8) {
                t[0] = (Q + ETA - a[8*i+0]) as u8;
                t[1] = (Q + ETA - a[8*i+1]) as u8;
                t[2] = (Q + ETA - a[8*i+2]) as u8;
                t[3] = (Q + ETA - a[8*i+3]) as u8;
                t[4] = (Q + ETA - a[8*i+4]) as u8;
                t[5] = (Q + ETA - a[8*i+5]) as u8;
                t[6] = (Q + ETA - a[8*i+6]) as u8;
                t[7] = (Q + ETA - a[8*i+7]) as u8;
    
                r[3*i+0]  = t[0];
                r[3*i+0] |= t[1] << 3;
                r[3*i+0] |= t[2] << 6;
                r[3*i+1]  = t[2] >> 2;
                r[3*i+1] |= t[3] << 1;
                r[3*i+1] |= t[4] << 4;
                r[3*i+1] |= t[5] << 7;
                r[3*i+2]  = t[5] >> 1;
                r[3*i+2] |= t[6] << 2;
                r[3*i+2] |= t[7] << 5;
            }
        } else {
            let mut t = [0; 2];
            for i in 0..(N / 2) {
                t[0] = (Q + ETA - a[2*i+0]) as u8;
                t[1] = (Q + ETA - a[2*i+1]) as u8;
                r[i] = t[0] | (t[1] << 4);
            }
        }
    }
    
    #[inline]
    pub fn eta_unpack(r: &mut Poly, a: &[u8; POLETA_SIZE_PACKED]) {
        if ETA <= 3 {
            for i in 0..(N / 8) {
                r[8*i+0] = u32::from(a[3*i+0]) & 0x07;
                r[8*i+1] = (u32::from(a[3*i+0]) >> 3) & 0x07;
                r[8*i+2] = (u32::from(a[3*i+0]) >> 6) | ((u32::from(a[3*i+1]) & 0x01) << 2);
                r[8*i+3] = (u32::from(a[3*i+1]) >> 1) & 0x07;
                r[8*i+4] = (u32::from(a[3*i+1]) >> 4) & 0x07;
                r[8*i+5] = (u32::from(a[3*i+1]) >> 7) | ((u32::from(a[3*i+2]) & 0x03) << 1);
                r[8*i+6] = (u32::from(a[3*i+2]) >> 2) & 0x07;
                r[8*i+7] = u32::from(a[3*i+2]) >> 5;
    
                r[8*i+0] = Q + ETA - r[8*i+0];
                r[8*i+1] = Q + ETA - r[8*i+1];
                r[8*i+2] = Q + ETA - r[8*i+2];
                r[8*i+3] = Q + ETA - r[8*i+3];
                r[8*i+4] = Q + ETA - r[8*i+4];
                r[8*i+5] = Q + ETA - r[8*i+5];
                r[8*i+6] = Q + ETA - r[8*i+6];
                r[8*i+7] = Q + ETA - r[8*i+7];
            }
        } else {
            for i in 0..(N / 2) {
                r[2*i+0] = u32::from(a[i]) & 0x0F;
                r[2*i+1] = u32::from(a[i]) >> 4;
                r[2*i+0] = Q + ETA - r[2*i+0];
                r[2*i+1] = Q + ETA - r[2*i+1];
            }
        }
    }
    
    #[inline]
    pub fn t0_pack(r: &mut [u8], a: &Poly) {
        let mut t = [0; 4];
        for i in 0..(N / 4) {
            t[0] = Q + (1 << (D-1) as u32) - a[4*i+0];
            t[1] = Q + (1 << (D-1) as u32) - a[4*i+1];
            t[2] = Q + (1 << (D-1) as u32) - a[4*i+2];
            t[3] = Q + (1 << (D-1) as u32) - a[4*i+3];
    
            r[7*i+0]  =  t[0] as u8;
            r[7*i+1]  =  (t[0] >> 8) as u8;
            r[7*i+1] |=  (t[1] << 6) as u8;
            r[7*i+2]  =  (t[1] >> 2) as u8;
            r[7*i+3]  =  (t[1] >> 10) as u8;
            r[7*i+3] |=  (t[2] << 4) as u8;
            r[7*i+4]  =  (t[2] >> 4) as u8;
            r[7*i+5]  =  (t[2] >> 12) as u8;
            r[7*i+5] |=  (t[3] << 2) as u8;
            r[7*i+6]  =  (t[3] >> 6) as u8;
        }
    }
    
    #[inline]
    pub fn t0_unpack(r: &mut Poly, a: &[u8]) {
        for i in 0..(N / 4) {
            r[4*i+0]  = u32::from(a[7*i+0]);
            r[4*i+0] |= (u32::from(a[7*i+1]) & 0x3F) << 8;
    
            r[4*i+1]  = u32::from(a[7*i+1]) >> 6;
            r[4*i+1] |= u32::from(a[7*i+2]) << 2;
            r[4*i+1] |= (u32::from(a[7*i+3]) & 0x0F) << 10;
    
            r[4*i+2]  = u32::from(a[7*i+3]) >> 4;
            r[4*i+2] |= u32::from(a[7*i+4]) << 4;
            r[4*i+2] |= (u32::from(a[7*i+5]) & 0x03) << 12;
    
            r[4*i+3]  = u32::from(a[7*i+5]) >> 2;
            r[4*i+3] |= u32::from(a[7*i+6]) << 6;
    
            r[4*i+0] = Q + (1 << (D-1) as u32) - r[4*i+0];
            r[4*i+1] = Q + (1 << (D-1) as u32) - r[4*i+1];
            r[4*i+2] = Q + (1 << (D-1) as u32) - r[4*i+2];
            r[4*i+3] = Q + (1 << (D-1) as u32) - r[4*i+3];
        }
    }
    
    #[inline]
    pub fn t1_pack(r: &mut [u8; POLT1_SIZE_PACKED], a: &Poly) {
        for i in 0..(N / 8) {
            r[9*i+0]  = ( a[8*i+0] & 0xFF) as u8;
            r[9*i+1]  = ((a[8*i+0] >> 8) | ((a[8*i+1] & 0x7F) << 1)) as u8;
            r[9*i+2]  = ((a[8*i+1] >> 7) | ((a[8*i+2] & 0x3F) << 2)) as u8;
            r[9*i+3]  = ((a[8*i+2] >> 6) | ((a[8*i+3] & 0x1F) << 3)) as u8;
            r[9*i+4]  = ((a[8*i+3] >> 5) | ((a[8*i+4] & 0x0F) << 4)) as u8;
            r[9*i+5]  = ((a[8*i+4] >> 4) | ((a[8*i+5] & 0x07) << 5)) as u8;
            r[9*i+6]  = ((a[8*i+5] >> 3) | ((a[8*i+6] & 0x03) << 6)) as u8;
            r[9*i+7]  = ((a[8*i+6] >> 2) | ((a[8*i+7] & 0x01) << 7)) as u8;
            r[9*i+8]  = ( a[8*i+7] >> 1) as u8;
        }
    }
    
    #[inline]
    pub fn t1_unpack(r: &mut Poly, a: &[u8; POLT1_SIZE_PACKED]) {
        for i in 0..(N / 8) {
            r[8*i+0] =  u32::from(a[9*i+0])       | (u32::from(a[9*i+1] & 0x01) << 8);
            r[8*i+1] = (u32::from(a[9*i+1]) >> 1) | (u32::from(a[9*i+2] & 0x03) << 7);
            r[8*i+2] = (u32::from(a[9*i+2]) >> 2) | (u32::from(a[9*i+3] & 0x07) << 6);
            r[8*i+3] = (u32::from(a[9*i+3]) >> 3) | (u32::from(a[9*i+4] & 0x0F) << 5);
            r[8*i+4] = (u32::from(a[9*i+4]) >> 4) | (u32::from(a[9*i+5] & 0x1F) << 4);
            r[8*i+5] = (u32::from(a[9*i+5]) >> 5) | (u32::from(a[9*i+6] & 0x3F) << 3);
            r[8*i+6] = (u32::from(a[9*i+6]) >> 6) | (u32::from(a[9*i+7] & 0x7F) << 2);
            r[8*i+7] = (u32::from(a[9*i+7]) >> 7) | (u32::from(a[9*i+8] & 0xFF) << 1);
        }
    }
    
    #[inline]
    pub fn z_pack(r: &mut [u8; POLZ_SIZE_PACKED], a: &Poly) {
        let mut t = [0; 2];
        for i in 0..(N / 2) {
            t[0] = (GAMMA1 - 1).wrapping_sub(a[2*i+0]);
            t[0] = t[0].wrapping_add(((t[0] as i32) >> 31) as u32 & Q);
            t[1] = (GAMMA1 - 1).wrapping_sub(a[2*i+1]);
            t[1] = t[1].wrapping_add(((t[1] as i32) >> 31) as u32 & Q);
    
            r[5*i+0]  = t[0] as u8;
            r[5*i+1]  = (t[0] >> 8) as u8;
            r[5*i+2]  = (t[0] >> 16) as u8;
            r[5*i+2] |= (t[1] << 4) as u8;
            r[5*i+3]  = (t[1] >> 4) as u8;
            r[5*i+4]  = (t[1] >> 12) as u8;
        }
    }
    
    #[inline]
    pub fn z_unpack(r: &mut Poly, a: &[u8; POLZ_SIZE_PACKED]) {
        for i in 0..(N / 2) {
            r[2*i+0]  = u32::from(a[5*i+0]);
            r[2*i+0] |= u32::from(a[5*i+1]) << 8;
            r[2*i+0] |= u32::from(a[5*i+2] & 0x0F) << 16;
    
            r[2*i+1]  = u32::from(a[5*i+2]) >> 4;
            r[2*i+1] |= u32::from(a[5*i+3]) << 4;
            r[2*i+1] |= u32::from(a[5*i+4]) << 12;
    
            r[2*i+0] = (GAMMA1 - 1).wrapping_sub(r[2*i+0]);
            r[2*i+0] = r[2*i+0].wrapping_add(((r[2*i+0] as i32) >> 31) as u32 & Q);
            r[2*i+1] = (GAMMA1 - 1).wrapping_sub(r[2*i+1]);
            r[2*i+1] = r[2*i+1].wrapping_add(((r[2*i+1] as i32) >> 31) as u32 & Q);
        }
    }
    
    #[inline]
    pub fn w1_pack(r: &mut [u8; POLW1_SIZE_PACKED], a: &Poly) {
        for i in 0..(N / 2) {
            r[i] = (a[2 * i] | (a[2 * i + 1] << 4)) as u8;
        }
    }
}

///////////////////////////////////////////////////////////////////////////// Original file polyvec.rs

mod polyvec {

    use crate::polyvec;
    use super::params::*;
    use super::poly::{
        self,
        Poly,
    };

    polyvec!(PolyVecL, L);
    polyvec!(PolyVecK, K);
    
    pub fn pointwise_acc_invmontgomery(w: &mut Poly, u: &PolyVecL, v: &PolyVecL) {
        let mut t = [0; N];
    
        poly::pointwise_invmontgomery(w, &u[0], &v[0]);
    
        for i in 1..L {
            poly::pointwise_invmontgomery(&mut t, &u[i], &v[i]);
            poly::add_assign(w, &t);
        }
    
        poly::reduce(w);
    }
    
    impl PolyVecK {
        pub fn power2round(&self, v0: &mut Self, v1: &mut Self) {
            for i in 0..K {
                poly::power2round(&self[i], &mut v0[i], &mut v1[i]);
            }
        }
    
        pub fn decompose(&self, v0: &mut Self, v1: &mut Self) {
            for i in 0..K {
                poly::decompose(&self[i], &mut v0[i], &mut v1[i]);
            }
        }
    }
    
    pub fn make_hint(h: &mut PolyVecK, u: &PolyVecK, v: &PolyVecK) -> usize {
        let mut s = 0;
        for i in 0..K {
            s += poly::make_hint(&u[i], &v[i], &mut h[i]);
        }
        s
    }
    
    pub fn use_hint(w: &mut PolyVecK, u: &PolyVecK, h: &PolyVecK) {
        for i in 0..K {
            poly::use_hint(&mut w[i], &u[i], &h[i]);
        }
    }
}

///////////////////////////////////////////////////////////////////////////// Original file reduce.rs

mod reduce {

    use super::params::*;

    pub fn montgomery_reduce(a: u64) -> u32 {
        let mut t = a.wrapping_mul(QINV as u64);
        t &= (1 << 32) - 1;
        t *= u64::from(Q);
        t += a;
        (t >> 32) as u32
    }
    
    pub fn reduce32(mut a: u32) -> u32 {
        let mut t = a & 0x7f_ffff;
        a >>= 23;
        t += (a << 13) - a;
        t
    }
    
    pub fn csubq(mut a: u32) -> u32 {
        a = a.wrapping_sub(Q as u32);
        let c = ((a as i32) >> 31) & Q as i32;
        a.wrapping_add(c as u32)
    }
    
    pub fn freeze(a: u32) -> u32 {
        let a = reduce32(a);
        let a = csubq(a);
        a
    }
}

//////////////////////////////////////////////////////////////////////////// Original file rounding.rs

mod rounding {

    use super::params::*;
    
    pub fn power2round(a: u32) -> (u32, u32) {
        let d = D as u32;
    
        let mut t = (a & ((1 << d) - 1)) as i32;
        t -= (1 << (d - 1)) + 1;
        t += (t >> 31) & (1 << d);
        t -= (1 << (d - 1)) - 1;
        (Q.wrapping_add(t as u32), a.wrapping_sub(t as u32) >> d)
    }
    
    pub fn decompose(mut a: u32) -> (u32, u32) {
        let alpha = ALPHA as i32;
    
        let mut t = (a & 0x7_ffff) as i32;
        t += ((a >> 19) << 9) as i32;
        t -= alpha / 2 + 1;
        t += (t >> 31) & alpha;
        t -= alpha / 2 - 1;
        a = a.wrapping_sub(t as u32);
    
        let mut u = (a as i32) - 1;
        u >>= 31;
        a = (a >> 19) + 1;
        a -= (u & 1) as u32;
    
        (Q.wrapping_add(t as u32).wrapping_sub(a >> 4), a & 0xf)
    }
    
    pub fn make_hint(a: u32, b: u32) -> u32 {
        let (_, x) = decompose(a);
        let (_, y) = decompose(b);
        if x != y { 1 } else { 0 }
    }
    
    pub fn use_hint(a: u32, hint: u32) -> u32 {
        let (a0, a1) = decompose(a);
    
        if hint == 0 {
            a1
        } else if a0 > Q {
            a1.wrapping_add(1) & 0xf
        } else {
            a1.wrapping_sub(1) & 0xf
        }
    }
}

//////////////////////////////////////////////////////////////////////////// Original file sign.rs

pub mod sign {

    use crate::{
        shake128,
        shake256,
    };
    use super::params::*;
    use super::polyvec::{
        self,
        PolyVecL,
        PolyVecK,
    };
    use super::poly::{
        self,
        Poly,
    };
    use super::packing;

    use arrayref::{
        array_mut_ref,
        array_ref,
    };
    use byteorder::{
        ByteOrder,
        LittleEndian,
    };
    use digest::{
        Input,
        ExtendableOutput,
        XofReader,
    };
    use rand_core_old::{
        RngCore,
        CryptoRng,
    };
    use sha3::Shake256;
    
    pub(crate) fn expand_mat(
        mat:    &mut [PolyVecL; K],
        rho:    &[u8; SEEDBYTES],
    ) {
        const SHAKE128_RATE: usize = 168;
    
        let mut outbuf = [0; 5 * SHAKE128_RATE];
    
        for i in 0..K {
            for j in 0..L {
                shake128!(&mut outbuf; rho, &[(i + (j << 4)) as u8]);
                poly::uniform(&mut mat[i][j], &outbuf);
            }
        }
    }
    
    pub(crate) fn challenge(
        c:  &mut Poly,
        mu: &[u8; CRHBYTES],
        w1: &PolyVecK,
    ) {
        const SHAKE256_RATE: usize = 136;
    
        let mut outbuf = [0; SHAKE256_RATE];
        let mut w1pack = [0; K * POLW1_SIZE_PACKED];
        for (i, pack) in w1pack.chunks_mut(POLW1_SIZE_PACKED).enumerate() {
            let pack = array_mut_ref!(pack, 0, POLW1_SIZE_PACKED);
            poly::w1_pack(pack, &w1[i]);
        }
    
        let mut hasher = Shake256::default();
        hasher.process(mu);
        hasher.process(&w1pack);
        let mut xof = hasher.xof_result();
        xof.read(&mut outbuf);
    
        let signs = LittleEndian::read_u64(&outbuf);
        let mut pos = 8;
        let mut mask = 1;
    
        for i in 196..256 {
            let b = loop {
                if pos >= SHAKE256_RATE {
                    xof.read(&mut outbuf);
                    pos = 0;
                }
    
                let b = outbuf[pos] as usize;
                pos += 1;
                if b <= i { break b }
            };
    
            c[i] = c[b];
            c[b] = if signs & mask != 0 { Q - 1 } else { 1 };
            mask <<= 1;
        }
    }
    
    pub fn keypair<R: RngCore + CryptoRng>(
        rng:        &mut R,
        pk_bytes:   &mut [u8; PK_SIZE_PACKED],
        sk_bytes:   &mut [u8; SK_SIZE_PACKED],
    ) {
        let mut nonce = 0;
        let mut tr = [0; CRHBYTES];
        let mut seedbuf = [0; 3 * SEEDBYTES];
        let mut mat = [PolyVecL::default(); K];
        let mut s1 = PolyVecL::default();
        let (mut s2, mut t, mut t0, mut t1) = (
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
        );
    
        // Expand 32 bytes of randomness into rho, rhoprime and key
        rng.fill_bytes(&mut seedbuf[..SEEDBYTES]);
        shake256!(&mut seedbuf; &seedbuf[..SEEDBYTES]);
        let rho = array_ref!(seedbuf, 0, SEEDBYTES);
        let rhoprime = array_ref!(seedbuf, SEEDBYTES, SEEDBYTES);
        let key = array_ref!(seedbuf, 2 * SEEDBYTES, SEEDBYTES);
    
        // Expand matrix
        expand_mat(&mut mat, rho);
    
        // Sample short vectors s1 and s2
        for i in 0..L {
            poly::uniform_eta(&mut s1[i], rhoprime, nonce);
            nonce += 1;
        }
        for i in 0..K {
            poly::uniform_eta(&mut s2[i], rhoprime, nonce);
            nonce += 1;
        }
    
        // Matrix-vector multiplication
        let mut s1hat = s1.clone();
        s1hat.ntt();
        for i in 0..K {
            polyvec::pointwise_acc_invmontgomery(&mut t[i], &mat[i], &s1hat);
            poly::reduce(&mut t[i]);
            poly::invntt_montgomery(&mut t[i])
        }
    
        // Add noise vector s2
        t.add_assign(&s2);
    
        // Extract t1 and write public key
        t.freeze();
        t.power2round(&mut t0, &mut t1);
        packing::pk::pack(
            pk_bytes,
            rho,
            &t1,
        );
    
        // Compute CRH(rho, t1) and write secret key
        shake256!(&mut tr; pk_bytes);
        packing::sk::pack(
            sk_bytes,
            rho,
            key,
            &tr,
            &s1,
            &s2,
            &t0,
        );
    }
    
    pub fn sign(
        m:  &[u8],
        sk: &[u8; SK_SIZE_PACKED],
    )
        -> [u8; SIG_SIZE_PACKED]
    {
        let mut sig = [0; SIG_SIZE_PACKED];
        sign_mut(&mut sig, &m, &sk);
        sig
    }

    pub fn sign_mut(
        sig:    &mut [u8; SIG_SIZE_PACKED],
        m:      &[u8],
        sk:     &[u8; SK_SIZE_PACKED],
    ) {
        let mut nonce = 0;
        let mut mat = [PolyVecL::default(); K];
        let (mut s1, mut y, mut z) = (
            PolyVecL::default(),
            PolyVecL::default(),
            PolyVecL::default(),
        );
        let (mut s2, mut t0, mut w, mut w1) = (
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
        );
        let (mut h, mut wcs2, mut wcs20, mut ct0, mut tmp) = (
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
            PolyVecK::default(),
        );
        let (mut rho, mut key, mut mu) = (
            [0; SEEDBYTES],
            [0; SEEDBYTES],
            [0; CRHBYTES],
        );
    
        packing::sk::unpack(
            sk,
            &mut rho,
            &mut key,
            &mut mu,
            &mut s1,
            &mut s2,
            &mut t0,
        );
    
        // Compute CRH(tr, msg)
        shake256!(&mut mu; &mu, m);
    
        // Expand matrix and transform vectors
        expand_mat(&mut mat, &rho);
        s1.ntt();
        s2.ntt();
        t0.ntt();
    
        loop {
            let mut c = [0; N];
    
            // Sample intermediate vector
            for i in 0..L {
                poly::uniform_gamma1m1(&mut y[i], &key, &mu, nonce);
                nonce += 1;
            }
    
            // Matrix-vector multiplicatio
            let mut yhat = y.clone();
            yhat.ntt();
            for i in 0..K {
                polyvec::pointwise_acc_invmontgomery(&mut w[i], &mat[i], &yhat);
                poly::invntt_montgomery(&mut w[i]);
            }
    
            // Decompose w and call the random oracle
            w.csubq();
            w.decompose(&mut tmp, &mut w1);
            challenge(&mut c, &mu, &w1);
    
            // Compute z, reject if it reveals secret
            let mut chat = c.clone();
            poly::ntt(&mut chat);
            for i in 0..L {
                poly::pointwise_invmontgomery(&mut z[i], &chat, &s1[i]);
                poly::invntt_montgomery(&mut z[i])
            }
            z.add_assign(&y);
            z.freeze();
            if z.chknorm(GAMMA1 - BETA) { continue };
    
            // Compute w - cs2, reject if w1 can not be computed from it
            for i in 0..K {
                poly::pointwise_invmontgomery(&mut wcs20[i], &chat, &s2[i]);
                poly::invntt_montgomery(&mut wcs20[i]);
            }
            wcs2.with_sub(&w, &wcs20);
            wcs2.freeze();
            wcs2.decompose(&mut wcs20, &mut tmp);
            wcs20.csubq();
            if wcs20.chknorm(GAMMA2 - BETA) { continue };
    
            if tmp != w1 { continue };
    
            // Compute hints for w1
            for i in 0..K {
                poly::pointwise_invmontgomery(&mut ct0[i], &chat, &t0[i]);
                poly::invntt_montgomery(&mut ct0[i]);
            }
    
            ct0.csubq();
            if ct0.chknorm(GAMMA2) { continue };
    
            tmp.with_add(&wcs2, &ct0);
            tmp.csubq();
            let hint = polyvec::make_hint(&mut h, &wcs2, &tmp);
            if hint > OMEGA { continue };
    
            // Write signature
            packing::sign::pack(sig, &z, &h, &c);
    
            break
        }
    }
    
    pub fn verify(
        m:      &[u8],
        sig:    &[u8; SIG_SIZE_PACKED],
        pk:     &[u8; PK_SIZE_PACKED],
    )
        -> bool
    {
        let (mut rho, mut mu) = ([0; SEEDBYTES], [0; CRHBYTES]);
        let (mut c, mut cp) = ([0; N], [0; N]);
        let mut mat = [PolyVecL::default(); K];
        let mut z = PolyVecL::default();
        let (mut t1, mut w1, mut h) = Default::default();
        let (mut tmp1, mut tmp2) = (PolyVecK::default(), PolyVecK::default());
    
        packing::pk::unpack(pk, &mut rho, &mut t1);
        let r = packing::sign::unpack(sig, &mut z, &mut h, &mut c);
    
        if !r { return false };
        if z.chknorm(GAMMA1 - BETA) { return false };
    
        // TODO
        // Compute CRH(CRH(rho, t1), msg)
        shake256!(&mut mu; pk);
        shake256!(&mut mu; &mu, m);
    
        // Matrix-vector multiplication; compute Az - c2^dt1
        expand_mat(&mut mat, &rho);
        z.ntt();
        for i in 0..K {
            polyvec::pointwise_acc_invmontgomery(&mut tmp1[i], &mat[i], &z);
        }
    
        let mut chat = c.clone();
        poly::ntt(&mut chat);
        t1.shift_left(D as u32);
        t1.ntt();
        for i in 0..K {
            poly::pointwise_invmontgomery(&mut tmp2[i], &chat, &t1[i]);
        }
    
        let mut tmp = PolyVecK::default();
        tmp.with_sub(&tmp1, &tmp2);
        tmp.reduce();
        tmp.invntt_montgomery();
    
        // Reconstruct w1
        tmp.csubq();
        polyvec::use_hint(&mut w1, &tmp, &h);
    
        // Call random oracle and verify challenge
        challenge(&mut cp, &mu, &w1);
    
        // TODO use subtle
        //  https://github.com/isislovecruft/subtle/pull/5
        (0..N)
            .map(|i| c[i] ^ cp[i])
            .fold(0, |sum, next| sum | next)
            .eq(&0)
    }
}

////////////////////////////////////////////////////////////////////////////// Original file test_mul.rs

#[cfg(test)]
mod test_mul {
    use super::*;
    use poly::Poly;
    use params::{ N, Q };
    use rand_core_old::{
        OsRng as OsRng_old,
        RngCore,
    };
    
    const NTESTS: usize = 10000;
    
    fn poly_naivemul(c: &mut Poly, a: &Poly, b: &Poly) {
        let mut r = [0; 2 * N];
    
        for i in 0..N {
            for j in 0..N {
                r[i + j] += ((u64::from(a[i]) * u64::from(b[j])) % u64::from(Q)) as u32;
                r[i + j] %= Q;
            }
        }
    
        for i in N..(2 * N) {
            r[i - N] = r[i - N] + (Q as u32) - r[i];
            r[i - N] %= Q;
        }
    
        c.copy_from_slice(&r[..N]);
    }
    
    
    #[test]
    fn test_dilithium_mul() {
        let mut rndbuf = [0; 840];
        let (mut c1, mut c2) = ([0; N], [0; N]);
        let (mut a, mut b) = ([0; N], [0; N]);
    
        for _ in 0..NTESTS {
            OsRng_old.fill_bytes(&mut rndbuf);
            poly::uniform(&mut a, &rndbuf);
            OsRng_old.fill_bytes(&mut rndbuf);
            poly::uniform(&mut b, &rndbuf);
    
            poly_naivemul(&mut c1, &a, &b);
    
            poly::ntt(&mut a);
            poly::ntt(&mut b);
            poly::pointwise_invmontgomery(&mut c2, &a, &b);
            poly::invntt_montgomery(&mut c2);
            poly::csubq(&mut c2);
    
            assert_eq!(&c2[..], &c1[..]);
        }
    }
}
/////////////////////////////////////////////////////////////////////////// Original file test_vectors.rs
#[cfg(test)]
mod test_vectors {
    use hex::{self, FromHexError};
    use byteorder::{ ByteOrder, BigEndian };
    use itertools::Itertools;
    use super::poly;
    use super::polyvec::{ self, PolyVecL, PolyVecK };
    use super::params::{
        N, K, L,
        SEEDBYTES, CRHBYTES
    };
    use super::sign;
    
    const TEST_VECTORS: &str = include_str!("../../tests/testvectors.txt");
    
    struct TestVector {
        seed: ([u8; SEEDBYTES], [u8; CRHBYTES]),
        mat: [PolyVecL; K],
        s: PolyVecL,
        y: PolyVecL,
        w1: PolyVecK,
        c: [u32; N]
    }
    
    impl Default for TestVector {
        fn default() -> Self {
            TestVector {
                seed: ([0; SEEDBYTES], [0; CRHBYTES]),
                mat: [PolyVecL::default(); K],
                s: PolyVecL::default(),
                y: PolyVecL::default(),
                w1: PolyVecK::default(),
                c: [0; N]
            }
        }
    }
    
    fn parse_testvectors() -> Result<Vec<TestVector>, FromHexError> {
        let mut testvectors = Vec::new();
    
        for testvector in TEST_VECTORS.lines().chunks(7).into_iter() {
            let mut tv = TestVector::default();
    
            for (key, val) in testvector
                .map(|line| line.split('='))
                .map(|mut split| (split.next(), split.last()))
                .filter_map(|(key, val)| key.and_then(|key| val.map(|val| (key.trim(), val.trim()))))
            {
                match key {
                    "count" => (),
                    "seed" => {
                        let seed = hex::decode(val)?;
                        let (rho, mu) = seed.split_at(SEEDBYTES);
                        tv.seed.0.copy_from_slice(rho);
                        tv.seed.1.copy_from_slice(mu);
                    },
                    "mat" => {
                        let mut mat = [0; K * L * N];
                        let mut i = 0;
                        BigEndian::read_u32_into(&hex::decode(val)?, &mut mat);
                        for j in 0..K {
                            for k in 0..L {
                                for l in 0..N {
                                    tv.mat[j][k][l] = mat[i];
                                    i += 1;
                                }
                            }
                        }
                    },
                    "s" => {
                        let mut s = [0; L * N];
                        let mut i = 0;
                        BigEndian::read_u32_into(&hex::decode(val)?, &mut s);
                        for j in 0..L {
                            for k in 0..N {
                                tv.s[j][k] = s[i];
                                i += 1;
                            }
                        }
                    },
                    "y" => {
                        let mut y = [0; L * N];
                        let mut i = 0;
                        BigEndian::read_u32_into(&hex::decode(val)?, &mut y);
                        for j in 0..L {
                            for k in 0..N {
                                tv.y[j][k] = y[i];
                                i += 1;
                            }
                        }
                    },
                    "w1" => {
                        let mut w1 = [0; K * N];
                        let mut i = 0;
                        BigEndian::read_u32_into(&hex::decode(val)?, &mut w1);
                        for j in 0..K {
                            for k in 0..N {
                                tv.w1[j][k] = w1[i];
                                i += 1;
                            }
                        }
                    },
                    "c" => {
                        BigEndian::read_u32_into(&hex::decode(val)?, &mut tv.c);
                    },
                    _ => panic!()
                }
            }
    
            testvectors.push(tv);
        }
    
        Ok(testvectors)
    }
    
    #[test]
    fn test_dilithium_vectors() {
        for tv in parse_testvectors().unwrap() {
            let mut mat = [PolyVecL::default(); K];
            let mut s = PolyVecL::default();
            let mut y = PolyVecL::default();
            let mut w = PolyVecK::default();
            let mut w1 = PolyVecK::default();
            let mut tmp = PolyVecK::default();
            let mut c = [0; N];
    
            sign::expand_mat(&mut mat, &tv.seed.0);
            assert!(&mat == &tv.mat);
    
            for i in 0..L {
                poly::uniform_eta(&mut s[i], &tv.seed.0, i as u8);
            }
            assert!(&s == &tv.s);
    
            for i in 0..L {
                poly::uniform_gamma1m1(&mut y[i], &tv.seed.0, &tv.seed.1, i as u16);
            }
            assert!(&y == &tv.y);
    
            y.ntt();
            for i in 0..K {
                polyvec::pointwise_acc_invmontgomery(&mut w[i], &mat[i], &y);
                poly::invntt_montgomery(&mut w[i]);
            }
            w.csubq();
            w.decompose(&mut tmp, &mut w1);
            assert!(&w1 == &tv.w1);
    
            sign::challenge(&mut c, &tv.seed.1, &w1);
            assert!(&c[..] == &tv.c[..]);
        }
    }
}

#[cfg(test)]
mod test_sign {
    use super::*;
    use crate::{
        pqc::dilithium::{
            params::*,
            sign::{
                keypair,
                sign,
                verify,
            },
        },
        sign::SignatureScheme,
    };

    use oxedize_fe2o3_core::prelude::*;
    use oxedize_fe2o3_iop_crypto::sign::Signer;

    use std::time::Instant;

    use ed25519_dalek::{
        Signer as DalekSigner,
        SigningKey,
        Verifier,
    };
    use rand_core_old::{
        OsRng as OsRng_old,
        RngCore,
    };
    use rand_core::OsRng;
    
    const BATCH_SIZE: usize = 1000;
    const MSG_SIZE: usize = 32;

    #[test]
    fn test_rough_dilithium_speed_compare_00() {
        msg!("Constructing {} messages of size {} bytes...", BATCH_SIZE, MSG_SIZE);
        let mut msgs = Vec::new();
        for _ in 0..BATCH_SIZE {
            let mut msg = [0; MSG_SIZE];
            OsRng_old.fill_bytes(&mut msg);
            msgs.push(msg);
        }

        msg!("Creating Dilithium key pair...");
        let (mut pk, mut sk) = ([0; PUBLICKEYBYTES], [0; SECRETKEYBYTES]);
        keypair(&mut OsRng_old, &mut pk, &mut sk);
        let t0 = Instant::now();
        msg!("Sign and verify {} messages...", BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            let sig = sign(&msgs[i], &sk);
            assert!(verify(&msgs[i], &sig, &pk));
        }
        let dt = t0.elapsed().as_millis();
        msg!("Dilithium signatures and verifications:  {} [ms]/loop.",
            (dt as f64) / (BATCH_SIZE as f64));

        msg!("Creating ED25519 key pair...");
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let t0 = Instant::now();
        msg!("Sign and verify {} messages...", BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            let sig = signing_key.sign(&msgs[i]);
            assert!(verifying_key.verify(&msgs[i], &sig).is_ok());
        }
        let dt = t0.elapsed().as_millis();
        msg!("ED25519 signatures and verifications:  {} [ms]/loop.",
            (dt as f64) / (BATCH_SIZE as f64));
    }

    #[test]
    fn test_rough_dilithium_speed_compare_01() -> Outcome<()> {
        msg!("Constructing {} messages of size {} bytes...", BATCH_SIZE, MSG_SIZE);
        let mut msgs = Vec::new();
        for _ in 0..BATCH_SIZE {
            let mut msg = [0; MSG_SIZE];
            OsRng_old.fill_bytes(&mut msg);
            msgs.push(msg);
        }

        msg!("Creating Dilithium scheme...");
        //let scheme = res!(SignatureScheme::new_dilithium2()); // TODO Substantially faster, why?
        let scheme = SignatureScheme::new_dilithium2_fe2o3();
        let t0 = Instant::now();
        msg!("Sign and verify {} messages...", BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            let sig = res!(scheme.sign(&msgs[i]));
            assert!(res!(scheme.verify(&msgs[i], &sig)));
        }
        let dt = t0.elapsed().as_millis();
        msg!("Dilithium signatures and verifications:  {} [ms]/loop.",
            (dt as f64) / (BATCH_SIZE as f64));

        msg!("Creating ED25519 key pair...");
        let scheme = SignatureScheme::new_ed25519();
        let t0 = Instant::now();
        msg!("Sign and verify {} messages...", BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            let sig = res!(scheme.sign(&msgs[i]));
            assert!(res!(scheme.verify(&msgs[i], &sig)));
        }
        let dt = t0.elapsed().as_millis();
        msg!("ED25519 signatures and verifications:  {} [ms]/loop.",
            (dt as f64) / (BATCH_SIZE as f64));

        Ok(())
    }
}
