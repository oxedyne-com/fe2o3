use crate::{
    keys::Keys,
};

use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    keys::KeyManager,
};
use oxedize_fe2o3_namex::{
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
    aead::Aead,
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
}
