use crate::{
    keys::Keys,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_iop_crypto::{
    enc::Encrypter,
    keys::KeyManager,
};
use oxedyne_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

use std::{
    convert::TryFrom,
    fmt,
    str,
};

use aes_gcm::{
    aead::{
        Aead,
        Payload,
    },
    Aes256Gcm,
    KeyInit,
    Nonce,
};
use rand_core::{
    OsRng,
    RngCore,
};
use secrecy::{
    ExposeSecret,
    Secret,
};


#[derive(Clone)]
pub enum EncryptionScheme {
    // Symmetric
    AES_256_GCM(Keys<
        0,
        {Self::AES_256_GCM_SK_LEN},
    >),
}

impl fmt::Display for EncryptionScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for EncryptionScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AES_256_GCM(..) => write!(f, "AES-256-GCM"),
        }
    }
}
    
impl InNamex for EncryptionScheme {

    fn name_id(&self) -> Outcome<NamexId> {
	    Ok(match self {
            Self::AES_256_GCM(..) => 
                res!(NamexId::try_from("4IaH4F8elJw60EkIr2N9+S1avkvUDHX5IaH1GkEKoXQ=")),
        })
    }

    /// Version-dependent identifier for the encryption scheme.  The type of the identifier can
    /// change with verisons.  This offers a much more compact alternative to the 256 bit Namex
    /// id.
    fn local_id(&self) -> LocalId {
	    match self {
            Self::AES_256_GCM(..)   => LocalId(1),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        let ids = match gname {
            "schemes" => [
	            ("AES-256-GCM", "4IaH4F8elJw60EkIr2N9+S1avkvUDHX5IaH1GkEKoXQ="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for EncryptionScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl Encrypter for EncryptionScheme {

    fn encrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> {
        match self {
            Self::AES_256_GCM(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let sk = sks.expose_secret();
                    let mut nbyts = [0u8; Self::AES_256_GCM_NONCE_BYTES];
                    OsRng.fill_bytes(&mut nbyts);
                    let nonce = Nonce::from_slice(&nbyts[..]);
                    let cipher = res!(Aes256Gcm::new_from_slice(&sk[..]));
                    let mut cipherdata = match cipher.encrypt(nonce, data) {
                        Err(e) => return Err(err!(e,
                            "While encrypting input of {} byts using scheme {:?}.",
                            data.len(), self;
                        Encrypt)),
                        Ok(result) => result,
                    };
                    cipherdata.extend_from_slice(&nbyts[..]);
                    Ok(cipherdata)
                },
                _ => Err(err!(
                    "Require secret key to encrypt.";
                Missing, Configuration)),
            },
        }
    }

    fn decrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> {
        match self {
            Self::AES_256_GCM(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let sk = sks.expose_secret();
                    if data.len() < Self::AES_256_GCM_NONCE_BYTES {
                        return Err(err!(
                            "Data length only {}.  Data to be decrypted using \
                            this scheme must have a minimum length of {} bytes \
                            so as to include the appended nonce.",
                            data.len(), Self::AES_256_GCM_NONCE_BYTES;
                        Decrypt, Invalid, Input));
                    }
                    let datlen = data.len() - Self::AES_256_GCM_NONCE_BYTES;
                    let nonce = Nonce::from_slice(&data[datlen..]);
                    let cipher = res!(Aes256Gcm::new_from_slice(&sk[..]));
                    match cipher.decrypt(nonce, &data[..datlen]) {
                        Err(e) => Err(err!(e,
                            "While decrypting input of {} byts (minus nonce) \
                            using scheme {:?}.", datlen, self;
                        Decrypt)),
                        Ok(result) => Ok(result),
                    }
                },
                _ => Err(err!(
                    "Require secret key to decrypt.";
                Missing, Configuration)),
            },
        }
    }

    fn is_identity(&self) -> bool { false }
}

impl KeyManager for EncryptionScheme {

    /// Clone using the specified keys.
    fn clone_with_keys(&self, _pk: Option<&[u8]>, sk: Option<&[u8]>) -> Outcome<Self> {
        Ok(match self {
            Self::AES_256_GCM(..) => Self::AES_256_GCM(Keys {
                pk: None,
                sks: match sk {
                    Some(sk) => Some(Secret::new(
                        res!(<[u8; Self::AES_256_GCM_SK_LEN]>::try_from(&sk[..]))
                    )),
                    None => None,
                },
            }),
        })
    }

    fn get_public_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::AES_256_GCM(keys) => match &keys.pk {
                Some(k) => Some(&k[..]),
                None => None,
            },
        })
    }

    fn get_secret_key(&self) -> Outcome<Option<&[u8]>> {
        Ok(match self {
            Self::AES_256_GCM(keys) => match &keys.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(&sk[..])
                },
                None => None,
            },
        })
    }

    fn set_public_key(mut self, _pk: Option<&[u8]>) -> Outcome<Self> {
        match &mut self {
            Self::AES_256_GCM(..) => (),
        }
        Ok(self)
    }

    fn set_secret_key(mut self, sk: Option<&[u8]>) -> Outcome<Self> {
        match &mut self {
            Self::AES_256_GCM(keys) => keys.sks = match sk {
                Some(sk) => Some(Secret::new(
                    res!(<[u8; Self::AES_256_GCM_SK_LEN]>::try_from(&sk[..]))
                )),
                None => None,
            },
        }
        Ok(self)
    }
}

impl str::FromStr for EncryptionScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "AES-256-GCM" => Ok(Self::new_aes_256_gcm()),
            _ => Err(err!(
                "The encryption scheme '{}' is not recognised.", name;
            Invalid, Input)),
        }
    }
}

impl TryFrom<&LocalId> for EncryptionScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: &LocalId) -> std::result::Result<Self, Self::Error> {
        match *n {
            LocalId(1) => Ok(Self::new_aes_256_gcm()),
            _ => Err(err!(
                "The encryption scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl TryFrom<(&LocalId, &[u8])> for EncryptionScheme {
    type Error = Error<ErrTag>;

    fn try_from((n, sk): (&LocalId, &[u8])) -> std::result::Result<Self, Self::Error> {
        match *n {
            LocalId(1) => {
                let result = res!(Self::new_aes_256_gcm_with_key(sk));
                Ok(result)
            },
            _ => Err(err!(
                "The encryption scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl EncryptionScheme {

    pub const AES_256_GCM_NONCE_BYTES:  usize = 12;
    pub const AES_256_GCM_SK_LEN:       usize = 32;

    pub fn new_aes_256_gcm() -> Self {
        Self::AES_256_GCM(Keys::randef_sk_only())
    }

    pub fn new_aes_256_gcm_with_key(sk: &[u8]) -> Outcome<Self> {
        let sks = Some(Secret::new(res!(<[u8; Self::AES_256_GCM_SK_LEN]>::try_from(&sk[..]))));
        Ok(Self::AES_256_GCM(Keys::new(None, sks)))
    }

    /// The AES-GCM authentication tag, which follows the ciphertext.
    pub const AES_256_GCM_TAG_BYTES:    usize = 16;

    /// AES-256-GCM under a caller's key and nonce, binding `aad` as additional data. Answers
    /// `ciphertext || tag`, with the nonce left OUT, so the caller frames it where its own wire
    /// format puts it (WebCrypto's `encrypt({ name: 'AES-GCM', iv, additionalData })` answers the
    /// same bytes). [`Encrypter::encrypt`] appends a random nonce and binds nothing, which is a
    /// different format; this is the primitive a format that is not Hematite's own is built on.
    ///
    /// The nonce must never repeat under one key. Draw it at random, as
    /// [`Self::aes_256_gcm_nonce`] does, or derive it from the plaintext (a synthetic IV) where
    /// equal plaintexts may be seen to be equal.
    pub fn aes_256_gcm_seal(
        key:    &[u8],
        nonce:  &[u8],
        aad:    &[u8],
        plain:  &[u8],
    )
        -> Outcome<Vec<u8>>
    {
        let cipher = res!(Self::aes_256_gcm_cipher(key, nonce));
        match cipher.encrypt(Nonce::from_slice(nonce), Payload { msg: plain, aad }) {
            Ok(ct) => Ok(ct),
            Err(e) => Err(err!(e,
                "While sealing {} bytes with AES-256-GCM.", plain.len();
            Encrypt)),
        }
    }

    /// Opens what [`Self::aes_256_gcm_seal`] sealed, under the same key, nonce and `aad`.
    /// Fails on a wrong key, a changed byte or a different `aad`, which the tag cannot tell apart.
    pub fn aes_256_gcm_open(
        key:    &[u8],
        nonce:  &[u8],
        aad:    &[u8],
        sealed: &[u8],
    )
        -> Outcome<Vec<u8>>
    {
        if sealed.len() < Self::AES_256_GCM_TAG_BYTES {
            return Err(err!(
                "AES-256-GCM input of {} bytes is shorter than its {} byte tag.",
                sealed.len(), Self::AES_256_GCM_TAG_BYTES;
            Decrypt, Invalid, Input));
        }
        let cipher = res!(Self::aes_256_gcm_cipher(key, nonce));
        match cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad }) {
            Ok(pt) => Ok(pt),
            Err(e) => Err(err!(e,
                "AES-256-GCM refused {} bytes: a wrong key, a changed byte or different \
                additional data.", sealed.len();
            Decrypt)),
        }
    }

    /// A fresh random AES-GCM nonce.
    pub fn aes_256_gcm_nonce() -> [u8; Self::AES_256_GCM_NONCE_BYTES] {
        let mut n = [0u8; Self::AES_256_GCM_NONCE_BYTES];
        OsRng.fill_bytes(&mut n);
        n
    }

    fn aes_256_gcm_cipher(key: &[u8], nonce: &[u8]) -> Outcome<Aes256Gcm> {
        if key.len() != Self::AES_256_GCM_SK_LEN {
            return Err(err!(
                "An AES-256-GCM key is {} bytes, not {}.", Self::AES_256_GCM_SK_LEN, key.len();
            Invalid, Input, Key, Size));
        }
        if nonce.len() != Self::AES_256_GCM_NONCE_BYTES {
            return Err(err!(
                "An AES-256-GCM nonce is {} bytes, not {}.",
                Self::AES_256_GCM_NONCE_BYTES, nonce.len();
            Invalid, Input, Size));
        }
        Ok(res!(Aes256Gcm::new_from_slice(key)))
    }

    /// The increase in the length of the encrypted data due to inclusion of essential metadata
    /// (e.g. in the case of AES-GCM, the nonce).
    pub fn len_inc(&self) -> usize {
        match self {
            Self::AES_256_GCM(_) => Self::AES_256_GCM_NONCE_BYTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enc_scheme_aes_gcm_00() -> Outcome<()> {
        let aes = EncryptionScheme::new_aes_256_gcm();
        for len in (1..100_000usize).step_by(1_000) {
            let mut plain = vec![0u8; len];
            OsRng.fill_bytes(&mut plain);
            let encrypted = res!(aes.encrypt(&plain));
            let result = res!(aes.decrypt(&encrypted));
            assert_eq!(result, plain, "Length of plain data = {}", len);
        }
        Ok(())
    }

    // NIST GCM test vector (gcmEncryptExtIV256, Count 0 of the 96-bit IV, 128-bit PT/AAD set):
    // the bytes WebCrypto answers for the same inputs.
    #[test]
    fn test_aes_256_gcm_seal_known_answer() -> Outcome<()> {
        fn unhex(s: &str) -> Vec<u8> {
            (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
        }
        let key = unhex("92e11dcdaa866f5ce790fd24501f92509aacf4cb8b1339d50c9c1240935dd08b");
        let iv  = unhex("ac93a1a6145299bde902f21a");
        let pt  = unhex("2d71bcfa914e4ac045b2aa60955fad24");
        let aad = unhex("1e0889016f67601c8ebea4943bc23ad6");
        let ct  = res!(EncryptionScheme::aes_256_gcm_seal(&key, &iv, &aad, &pt));
        req!(ct, unhex("8995ae2e6df3dbf96fac7b7137bae67feca5aa77d51d4a0a14d9c51e1da474ab"));
        req!(res!(EncryptionScheme::aes_256_gcm_open(&key, &iv, &aad, &ct)), pt);
        Ok(())
    }

    #[test]
    fn test_aes_256_gcm_open_refuses_wrong_aad_and_tamper() -> Outcome<()> {
        let key = [7u8; 32];
        let iv = EncryptionScheme::aes_256_gcm_nonce();
        let ct = res!(EncryptionScheme::aes_256_gcm_seal(&key, &iv, b"purpose/a", b"hello"));
        req!(res!(EncryptionScheme::aes_256_gcm_open(&key, &iv, b"purpose/a", &ct)), b"hello".to_vec());
        req!(EncryptionScheme::aes_256_gcm_open(&key, &iv, b"purpose/b", &ct).is_err(), true);
        req!(EncryptionScheme::aes_256_gcm_open(&key, &iv, b"", &ct).is_err(), true);
        let mut bad = ct.clone();
        bad[0] ^= 1;
        req!(EncryptionScheme::aes_256_gcm_open(&key, &iv, b"purpose/a", &bad).is_err(), true);
        req!(EncryptionScheme::aes_256_gcm_open(&key, &iv, b"purpose/a", &ct[..15]).is_err(), true);
        req!(EncryptionScheme::aes_256_gcm_seal(&key[..31], &iv, b"", b"x").is_err(), true);
        req!(EncryptionScheme::aes_256_gcm_seal(&key, &iv[..11], b"", b"x").is_err(), true);
        Ok(())
    }
}
