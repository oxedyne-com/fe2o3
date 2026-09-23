//! Standard Base64 as specified by RFC 4648 §4: the `A-Za-z0-9+/` alphabet, `=`
//! padding to a whole number of four character quanta, and nothing else.
//!
//! This is the encoding a browser's `atob` and `btoa` speak, and the one every
//! wire format that says "base64" means -- DKIM's `b=` and `p=` tags, HTTP Basic
//! credentials, PEM bodies, data URLs, JSON payloads carrying bytes.
//!
//! It is deliberately separate from [`crate::base2x`], whose `BASE64` constant
//! shares the alphabet but not the padding scheme: Base2x always writes three
//! padding characters where RFC 4648 writes one or two, so a Base2x string does
//! not survive `atob` and an RFC 4648 string does not survive Base2x decoding.
//! Base2x is the right tool for a custom alphabet; this module is the right tool
//! for talking to anybody else.
//!
//! # Strictness
//!
//! [`decode`] rejects rather than guesses, because two decoders that disagree
//! about the same string are how a signature verifies on one side and not the
//! other. It refuses:
//!
//! - any length that is not a multiple of four (RFC 4648 §4 pads every encoding);
//! - any character outside the alphabet, including whitespace and the URL-safe
//!   `-` and `_` substitutes of RFC 4648 §5;
//! - `=` anywhere but as the last one or two characters;
//! - a final quantum whose unused bits are not zero, which RFC 4648 §3.5 permits
//!   a decoder to reject and which this one does.
//!
//! A caller holding input that legitimately contains whitespace -- a PEM block,
//! or a header value folded across lines -- must strip it before calling.
//!
//! # Unpadded base64url
//!
//! [`encode_url`] and [`decode_url`] speak RFC 4648 §5 with the padding left
//! off, as JOSE, WebAuthn and most JSON protocols carry bytes: `-` and `_` in
//! place of `+` and `/`, and no `=`. [`decode_url`] is as strict as [`decode`]
//! and for the same reason. It refuses `=` anywhere, the `+` and `/` of the §4
//! alphabet, any other character outside the §5 alphabet, a length that leaves
//! one character over a whole number of quanta (six bits cannot make a byte),
//! and a final character whose unused bits are not zero, so every byte string
//! has exactly one accepted spelling.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;


// The RFC 4648 §4 alphabet, in index order.
const ALPHABET: [u8; 64] =
    *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// The RFC 4648 §5 URL and filename safe alphabet, in index order.
const URL_ALPHABET: [u8; 64] =
    *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

const PAD: u8 = b'=';

const INVALID: u8 = 0xFF; // marks a byte that is not in the alphabet, in DECODE

const DECODE:       [u8; 256] = decode_table(&ALPHABET);        // reverse of ALPHABET
const URL_DECODE:   [u8; 256] = decode_table(&URL_ALPHABET);    // reverse of URL_ALPHABET

const fn decode_table(alphabet: &[u8; 64]) -> [u8; 256] {
    let mut table = [INVALID; 256];
    let mut i = 0;
    while i < alphabet.len() {
        table[alphabet[i] as usize] = i as u8;
        i += 1;
    }
    table
}

/// Padding is included in the count.
pub fn encoded_len(n: usize) -> usize {
    ((n + 2) / 3) * 4
}

/// The length of the unpadded base64url encoding of `n` bytes.
pub fn encoded_url_len(n: usize) -> usize {
    (n * 4 + 2) / 3
}

/// Encodes bytes as standard, padded RFC 4648 §4 Base64.
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_text::base64;
///
/// assert_eq!(base64::encode(b"foobar"), "Zm9vYmFy");
/// assert_eq!(base64::encode(b"foo"), "Zm9v");
/// assert_eq!(base64::encode(b"fo"), "Zm8=");
/// ```
pub fn encode(data: &[u8]) -> String {
    encode_with(data, &ALPHABET, true)
}

/// Encodes bytes as unpadded RFC 4648 §5 base64url.
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_text::base64;
///
/// assert_eq!(base64::encode_url(b"foobar"), "Zm9vYmFy");
/// assert_eq!(base64::encode_url(b"fo"), "Zm8");
/// assert_eq!(base64::encode_url(&[0xfb, 0xff]), "-_8");
/// ```
pub fn encode_url(data: &[u8]) -> String {
    encode_with(data, &URL_ALPHABET, false)
}

fn encode_with(data: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity(encoded_len(data.len()));
    for chunk in data.chunks(3) {
        // The quantum, most significant byte first, zero filled when the input
        // runs out.
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let q = (b0 << 16) | (b1 << 8) | b2;

        out.push(alphabet[((q >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((q >> 12) & 0x3F) as usize] as char);
        match chunk.len() {
            1 => if pad {
                out.push(PAD as char);
                out.push(PAD as char);
            },
            2 => {
                out.push(alphabet[((q >> 6) & 0x3F) as usize] as char);
                if pad {
                    out.push(PAD as char);
                }
            },
            _ => {
                out.push(alphabet[((q >> 6) & 0x3F) as usize] as char);
                out.push(alphabet[(q & 0x3F) as usize] as char);
            },
        }
    }
    out
}

/// Decodes standard, padded RFC 4648 §4 Base64, refusing anything that is not.
///
/// See the module documentation for exactly what is refused.
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_text::base64;
///
/// assert!(base64::decode("Zm9vYmFy").is_ok());
/// assert!(base64::decode("Zm9").is_err());        // Not a whole quantum.
/// assert!(base64::decode("Zm9v Zg==").is_err());  // Whitespace is not alphabet.
/// ```
pub fn decode(s: &str) -> Outcome<Vec<u8>> {
    let src = s.as_bytes();
    let n = src.len();

    if n % 4 != 0 {
        return Err(err!(
            "Base64 input is {} characters long, which is not a multiple of 4; \
            RFC 4648 §4 pads every encoding out to whole 4 character quanta.", n;
        Invalid, Input, Decode, Size));
    }
    if n == 0 {
        return Ok(Vec::new());
    }

    // Padding is legal only as the last one or two characters. Counting it here
    // tells the final quantum how many bytes it carries; a '=' anywhere else is
    // caught by `sextet`, which refuses it like any other non-alphabet byte.
    let pad = if src[n - 1] == PAD {
        if src[n - 2] == PAD { 2 } else { 1 }
    } else {
        0
    };

    let mut out = Vec::with_capacity((n / 4) * 3 - pad);

    // Every quantum but the last carries three whole bytes.
    let last = n - 4;
    let mut i = 0;
    while i < last {
        let q = (res!(sextet(src, i)) << 18)
            | (res!(sextet(src, i + 1)) << 12)
            | (res!(sextet(src, i + 2)) << 6)
            | res!(sextet(src, i + 3));
        out.push((q >> 16) as u8);
        out.push((q >> 8) as u8);
        out.push(q as u8);
        i += 4;
    }

    // The last quantum, whose padding says how much of it is data.
    let a = res!(sextet(src, last));
    let b = res!(sextet(src, last + 1));
    match pad {
        2 => {
            // Two characters carry 12 bits for 1 byte, so 4 bits go unused.
            if b & 0x0F != 0 {
                return Err(err!(
                    "Base64 input ends with '{}{}==', whose final character sets \
                    bits the single decoded byte cannot hold; RFC 4648 §3.5 \
                    requires those bits to be zero.",
                    char::from(src[last]), char::from(src[last + 1]);
                Invalid, Input, Decode));
            }
            out.push(((a << 2) | (b >> 4)) as u8);
        },
        1 => {
            let c = res!(sextet(src, last + 2));
            // Three characters carry 18 bits for 2 bytes, so 2 bits go unused.
            if c & 0x03 != 0 {
                return Err(err!(
                    "Base64 input ends with '{}{}{}=', whose final character sets \
                    bits the 2 decoded bytes cannot hold; RFC 4648 §3.5 requires \
                    those bits to be zero.",
                    char::from(src[last]), char::from(src[last + 1]),
                    char::from(src[last + 2]);
                Invalid, Input, Decode));
            }
            out.push(((a << 2) | (b >> 4)) as u8);
            out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
        },
        _ => {
            let c = res!(sextet(src, last + 2));
            let d = res!(sextet(src, last + 3));
            out.push(((a << 2) | (b >> 4)) as u8);
            out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
            out.push((((c & 0x03) << 6) | d) as u8);
        },
    }

    Ok(out)
}

/// Decodes unpadded RFC 4648 §5 base64url, refusing anything that is not.
///
/// See the module documentation for exactly what is refused.
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_text::base64;
///
/// assert_eq!(base64::decode_url("-_8").ok(), Some(vec![0xfb, 0xff]));
/// assert!(base64::decode_url("Zm8=").is_err());   // Padding is not part of it.
/// assert!(base64::decode_url("+/8").is_err());    // Nor is the §4 alphabet.
/// ```
pub fn decode_url(s: &str) -> Outcome<Vec<u8>> {
    let src = s.as_bytes();
    let n = src.len();
    if n % 4 == 1 {
        return Err(err!(
            "Unpadded base64url input is {} characters long, which leaves one \
            character over a whole number of 4 character quanta, and one character \
            carries 6 bits, too few for a byte.", n;
        Invalid, Input, Decode, Size));
    }

    let mut out = Vec::with_capacity(n * 3 / 4);

    // Every whole quantum carries three bytes.
    let whole = n - n % 4;
    let mut i = 0;
    while i < whole {
        let q = (res!(url_sextet(src, i)) << 18)
            | (res!(url_sextet(src, i + 1)) << 12)
            | (res!(url_sextet(src, i + 2)) << 6)
            | res!(url_sextet(src, i + 3));
        out.push((q >> 16) as u8);
        out.push((q >> 8) as u8);
        out.push(q as u8);
        i += 4;
    }

    // A short final quantum, which the missing padding would have filled out.
    match n - whole {
        2 => {
            let a = res!(url_sextet(src, whole));
            let b = res!(url_sextet(src, whole + 1));
            // Two characters carry 12 bits for 1 byte, so 4 bits go unused.
            if b & 0x0F != 0 {
                return Err(err!(
                    "Base64url input ends with '{}{}', whose final character sets \
                    bits the single decoded byte cannot hold; RFC 4648 §3.5 \
                    requires those bits to be zero.",
                    char::from(src[whole]), char::from(src[whole + 1]);
                Invalid, Input, Decode));
            }
            out.push(((a << 2) | (b >> 4)) as u8);
        },
        3 => {
            let a = res!(url_sextet(src, whole));
            let b = res!(url_sextet(src, whole + 1));
            let c = res!(url_sextet(src, whole + 2));
            // Three characters carry 18 bits for 2 bytes, so 2 bits go unused.
            if c & 0x03 != 0 {
                return Err(err!(
                    "Base64url input ends with '{}{}{}', whose final character sets \
                    bits the 2 decoded bytes cannot hold; RFC 4648 §3.5 requires \
                    those bits to be zero.",
                    char::from(src[whole]), char::from(src[whole + 1]),
                    char::from(src[whole + 2]);
                Invalid, Input, Decode));
            }
            out.push(((a << 2) | (b >> 4)) as u8);
            out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
        },
        _ => (),
    }

    Ok(out)
}

/// The 6 bit value of the base64url character at `i`.
fn url_sextet(src: &[u8], i: usize) -> Outcome<u32> {
    let c = src[i];
    let v = URL_DECODE[c as usize];
    if v == INVALID {
        let why = match c {
            PAD         => "padding, which unpadded base64url never carries",
            b'+' | b'/' => "from the RFC 4648 §4 alphabet, not the §5 one",
            _           => "not in the RFC 4648 §5 alphabet",
        };
        return Err(err!(
            "Base64url input has '{}' (byte 0x{:02x}) at index {} of {}, which is {}.",
            char::from(c).escape_default(), c, i, src.len(), why;
        Invalid, Input, Decode));
    }
    Ok(v as u32)
}

/// The 6 bit value of the alphabet character at `i`.
fn sextet(src: &[u8], i: usize) -> Outcome<u32> {
    let c = src[i];
    let v = DECODE[c as usize];
    if v == INVALID {
        if c == PAD {
            return Err(err!(
                "Base64 input has padding '=' at index {} of {}, but RFC 4648 §4 \
                allows it only as the last one or two characters.", i, src.len();
            Invalid, Input, Decode));
        }
        return Err(err!(
            "Base64 input has '{}' (byte 0x{:02x}) at index {}, which is not in \
            the RFC 4648 §4 alphabet.", char::from(c).escape_default(), c, i;
        Invalid, Input, Decode));
    }
    Ok(v as u32)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10 lists these seven encodings verbatim. They are the oracle:
    /// they did not come from this implementation, and an implementation that is
    /// wrong in a self-consistent way still fails them.
    const RFC4648_VECTORS: [(&str, &str); 7] = [
        ("",        ""),
        ("f",       "Zg=="),
        ("fo",      "Zm8="),
        ("foo",     "Zm9v"),
        ("foob",    "Zm9vYg=="),
        ("fooba",   "Zm9vYmE="),
        ("foobar",  "Zm9vYmFy"),
    ];

    #[test]
    fn test_the_rfc_4648_test_vectors_encode_as_the_rfc_says() {
        for (plain, encoded) in RFC4648_VECTORS {
            assert_eq!(encode(plain.as_bytes()), encoded, "encoding {:?}", plain);
        }
    }

    #[test]
    fn test_the_rfc_4648_test_vectors_decode_as_the_rfc_says() -> Outcome<()> {
        for (plain, encoded) in RFC4648_VECTORS {
            let got = res!(decode(encoded));
            assert_eq!(got, plain.as_bytes(), "decoding {:?}", encoded);
        }
        Ok(())
    }

    /// Fixtures produced by an independent tool, so that agreement is agreement
    /// with somebody else. Each was generated with GNU-compatible `base64(1)`:
    ///
    /// ```text
    /// $ printf 'Hematite' | base64
    /// SGVtYXRpdGU=
    /// $ printf '\x00' | base64
    /// AA==
    /// $ printf '\xff\xff' | base64
    /// //8=
    /// $ printf '\x00\x01\x02\xfd\xfe\xff' | base64
    /// AAEC/f7/
    /// $ printf '\x00\x01\x02 ... \x1f' | base64 -w0
    /// AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=
    /// ```
    #[test]
    fn test_an_external_tool_agrees_with_this_encoder() -> Outcome<()> {
        let cases: [(Vec<u8>, &str); 5] = [
            (b"Hematite".to_vec(),                      "SGVtYXRpdGU="),
            (vec![0x00],                                "AA=="),
            (vec![0xFF, 0xFF],                          "//8="),
            (vec![0x00, 0x01, 0x02, 0xFD, 0xFE, 0xFF],  "AAEC/f7/"),
            ((0u8..32).collect::<Vec<u8>>(),
                "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="),
        ];
        for (bytes, expected) in cases {
            assert_eq!(encode(&bytes), expected, "encoding {:?}", bytes);
            assert_eq!(res!(decode(expected)), bytes, "decoding {:?}", expected);
        }
        Ok(())
    }

    /// Binary is not text: the bytes a codec is most likely to mangle are the
    /// ones no string contains, so every length up to 3 quanta of every byte
    /// value goes round.
    #[test]
    fn test_arbitrary_bytes_survive_a_round_trip() -> Outcome<()> {
        for len in 0..=12 {
            for start in 0..=255u16 {
                let bytes: Vec<u8> = (0..len)
                    .map(|k| ((start as usize + k * 37) % 256) as u8)
                    .collect();
                let encoded = encode(&bytes);
                assert_eq!(encoded.len(), encoded_len(bytes.len()));
                let got = res!(decode(&encoded));
                assert_eq!(got, bytes, "round trip of {:?} via {:?}", bytes, encoded);
            }
        }
        Ok(())
    }

    /// A run of every byte value, including the 0x00 and 0xFF that a text-shaped
    /// implementation trips over.
    #[test]
    fn test_every_byte_value_survives_a_round_trip() -> Outcome<()> {
        let bytes: Vec<u8> = (0..=255u8).collect();
        let encoded = encode(&bytes);
        assert_eq!(res!(decode(&encoded)), bytes);
        Ok(())
    }

    /// Rejection is the whole point of a strict decoder: a decoder that guesses
    /// is a decoder that disagrees with the one at the other end.
    #[test]
    fn test_malformed_input_is_refused() {
        let bad = [
            ("Zm9",         "length not a multiple of 4"),
            ("Z",           "length not a multiple of 4"),
            ("Zm9vYmFyZ",   "length not a multiple of 4"),
            ("Zm9$",        "character outside the alphabet"),
            ("Zm9v Zg==",   "space is not in the alphabet"),
            ("Zm9v\nZg==",  "newline is not in the alphabet"),
            ("Zm-_",        "URL-safe alphabet is a different encoding"),
            ("Zmé",         "non-ASCII is not in the alphabet"),
            ("Zm=vYg==",    "padding before the last quantum"),
            ("=m9vYg==",    "padding at the start of a quantum"),
            ("Zg=A",        "data after padding"),
            ("Z===",        "three padding characters"),
            ("====",        "a quantum of nothing but padding"),
            ("Zh==",        "final character sets bits the byte cannot hold"),
            ("Zm9=",        "final character sets bits the bytes cannot hold"),
        ];
        for (input, why) in bad {
            assert!(decode(input).is_err(), "accepted {:?}, but {}", input, why);
        }
    }

    /// The canonical forms of those same shapes are accepted, so the rejections
    /// above are about the fault and not about the neighbourhood.
    #[test]
    fn test_the_canonical_neighbours_are_accepted() -> Outcome<()> {
        for good in ["Zg==", "Zm8=", "Zm9v", "AA==", "AQ==", "//8="] {
            res!(decode(good));
        }
        Ok(())
    }

    /// An independent implementation, the `base64` crate, over a spread of
    /// lengths and byte values. It is a dev-dependency only: the point of this
    /// module is that the library dependency can go, and the point of this test
    /// is to show that dropping it changes nothing on the wire.
    #[test]
    fn test_an_independent_implementation_agrees() -> Outcome<()> {
        // A cheap deterministic spread, so a failure is reproducible.
        let mut state: u32 = 0x1234_5678;
        for len in 0..200 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect();
            let ours = encode(&bytes);
            let theirs = ::base64::encode(&bytes);
            assert_eq!(ours, theirs, "encoding {} bytes: {:?}", len, bytes);
            assert_eq!(res!(decode(&theirs)), bytes, "decoding {:?}", theirs);
        }
        Ok(())
    }

    /// A length calculation that disagrees with the encoder is a buffer that is
    /// resized on every call.
    #[test]
    fn test_the_predicted_length_matches_the_encoding() {
        for len in 0..=64 {
            let bytes = vec![0xA5u8; len];
            assert_eq!(encode(&bytes).len(), encoded_len(len), "for {} bytes", len);
            assert_eq!(encode_url(&bytes).len(), encoded_url_len(len), "for {} bytes", len);
        }
    }

    /// RFC 4648 §10's vectors with the padding taken off. None of them reaches
    /// the two characters where §5 departs from §4, so dropping the padding is
    /// the whole of the difference.
    const RFC4648_URL_VECTORS: [(&str, &str); 7] = [
        ("",        ""),
        ("f",       "Zg"),
        ("fo",      "Zm8"),
        ("foo",     "Zm9v"),
        ("foob",    "Zm9vYg"),
        ("fooba",   "Zm9vYmE"),
        ("foobar",  "Zm9vYmFy"),
    ];

    #[test]
    fn test_the_rfc_4648_vectors_unpadded_go_both_ways() -> Outcome<()> {
        for (plain, encoded) in RFC4648_URL_VECTORS {
            assert_eq!(encode_url(plain.as_bytes()), encoded, "encoding {:?}", plain);
            assert_eq!(res!(decode_url(encoded)), plain.as_bytes(), "decoding {:?}", encoded);
        }
        Ok(())
    }

    /// `-` and `_` stand exactly where the standard encoding has `+` and `/`.
    #[test]
    fn test_the_url_alphabet_replaces_plus_and_slash() -> Outcome<()> {
        assert_eq!(encode(&[0xfb, 0xff]), "+/8=");
        assert_eq!(encode_url(&[0xfb, 0xff]), "-_8");
        assert_eq!(res!(decode_url("-_8")), vec![0xfb, 0xff]);
        assert_eq!(encode_url(&[0xff]), "_w");
        assert_eq!(res!(decode_url("_w")), vec![0xff]);
        Ok(())
    }

    /// The `base64` crate's `URL_SAFE_NO_PAD`, an independent implementation,
    /// over a spread of lengths and byte values.
    #[test]
    fn test_an_independent_implementation_agrees_on_base64url() -> Outcome<()> {
        let mut state: u32 = 0x8765_4321;
        for len in 0..200 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect();
            let ours = encode_url(&bytes);
            let theirs = ::base64::encode_config(&bytes, ::base64::URL_SAFE_NO_PAD);
            assert_eq!(ours, theirs, "encoding {} bytes: {:?}", len, bytes);
            assert_eq!(res!(decode_url(&theirs)), bytes, "decoding {:?}", theirs);
        }
        Ok(())
    }

    /// Every byte string has one accepted spelling, so everything else is
    /// refused rather than read the way some other decoder might read it.
    #[test]
    fn test_malformed_base64url_is_refused() {
        let bad = [
            ("Z",           "one character over a whole quantum"),
            ("Zm9vY",       "one character over a whole quantum"),
            ("Zg==",        "padding"),
            ("Zm8=",        "padding"),
            ("Zm9v+g",      "the plus of the standard alphabet"),
            ("Zm9v/g",      "the slash of the standard alphabet"),
            ("Zm9v Zg",     "space is not in the alphabet"),
            ("Zm9v\nZg",    "newline is not in the alphabet"),
            ("Zm9$",        "a character in neither alphabet"),
            ("Zmé",         "non-ASCII is not in the alphabet"),
            ("Zh",          "final character sets bits the byte cannot hold"),
            ("Zm9",         "final character sets bits the bytes cannot hold"),
        ];
        for (input, why) in bad {
            assert!(decode_url(input).is_err(), "accepted {:?}, but {}", input, why);
        }
    }

    /// The canonical forms of those same shapes are accepted, so the refusals
    /// above are about the fault and not about the neighbourhood.
    #[test]
    fn test_the_canonical_base64url_neighbours_are_accepted() -> Outcome<()> {
        for good in ["", "Zg", "Zm8", "Zm9v", "AA", "AQ", "_w", "-_8"] {
            res!(decode_url(good));
        }
        Ok(())
    }
}
