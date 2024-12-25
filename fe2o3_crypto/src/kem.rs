use crate::{
    pqc::saber::{
        self,
        SaberAlgorithm,
    },
    keys::Keys,
};

use oxedize_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedize_fe2o3_iop_crypto::{
    kem::KeyExchanger,
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
    fmt::{
        self,
        Debug,
    },
    str,
};

use secrecy::{
    ExposeSecret,
};


#[derive(Clone)]
pub enum KeyExchangeScheme {
    FireSaber(Keys<
        {Self::FIRESABER_PK_LEN},
        {Self::FIRESABER_SK_LEN},
    >),
}

impl Debug for KeyExchangeScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FireSaber(..) => write!(f, "FireSaber-KEM"),
        }
    }
}
    
impl InNamex for KeyExchangeScheme {

    fn name_id(&self) -> Outcome<NamexId> {
	    Ok(match self {
            Self::FireSaber(..) =>
                res!(NamexId::try_from("nb68Os+ihmMixsBrsV5K+OzgCJrtxQ1L3e1FJ7KOvPk=")),
        })
    }

    /// Version-dependent identifier for the encryption scheme.  The type of the identifier can
    /// change with verisons.  This offers a much more compact alternative to the 256 bit Namex
    /// id.
    fn local_id(&self) -> LocalId {
	    match self {
            Self::FireSaber(..)     => LocalId(1),
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
                ("FireSaber", "nb68Os+ihmMixsBrsV5K+OzgCJrtxQ1L3e1FJ7KOvPk="),
            ],
            _ => return Err(err!(errmsg!(
                "The Namex group name '{}' is not recognised for KeyExchangeScheme.", gname,
            ), Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl KeyExchanger for KeyExchangeScheme {

    fn encap<
        const PK_LEN: usize,
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        _pk: [u8; PK_LEN], // TODO does the trait need this?
    )
        -> Outcome<(
            [u8; SESSION_KEY_LEN],
            [u8; CIPHERTEXT_LEN],
        )>
    {
        match self {
            Self::FireSaber(keys) => match keys {
                Keys { pk: Some(pk_byts), .. } => { 
                    let scheme = saber::FireSaber;
                    let pk = res!(saber::PublicKey::from_bytes(&pk_byts[..]));
                    let (sess_key1, ct1) = scheme.kem_encap(&pk);
                    let ct2 = ct1.to_bytes::<{CIPHERTEXT_LEN}>();
                    let ct3 = res!(<[u8; CIPHERTEXT_LEN]>::try_from(&ct2[..]));
                    let sess_key2 = res!(<[u8; SESSION_KEY_LEN]>::try_from(&sess_key1[..]));
                    Ok((sess_key2, ct3))
                },
                _ => Err(err!(errmsg!(
                    "Require public key to encapsulate.",
                ), Missing, Configuration)),
            },
        }
    }

    fn decap<
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        ciphertext: [u8; CIPHERTEXT_LEN],
    )
        -> Outcome<[u8; SESSION_KEY_LEN]>
    {
        match self {
            Self::FireSaber(keys) => match keys {
                Keys { sks: Some(sks), .. } => { 
                    let sk_byts = sks.expose_secret();
                    let scheme = saber::FireSaber;
                    let sk_cca = res!(saber::SecretKeyCCA::from_bytes(&sk_byts[..]));
                    let sess_key1 = res!(scheme.kem_decap(&ciphertext, &sk_cca));
                    let sess_key2 = res!(<[u8; SESSION_KEY_LEN]>::try_from(&sess_key1[..]));
                    Ok(sess_key2)
                },
                _ => Err(err!(errmsg!(
                    "Require secret key to de-encapsulate.",
                ), Missing, Configuration)),
            },
        }
    }
}

impl str::FromStr for KeyExchangeScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "FireSaber" => Ok(Self::new_firesaber()),
            _ => Err(err!(errmsg!(
                "The key exchange scheme '{}' is not recognised.", name,
            ), Invalid, Input)),
        }
    }
}

impl TryFrom<LocalId> for KeyExchangeScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: LocalId) -> std::result::Result<Self, Self::Error> {
        match n {
            LocalId(1) => Ok(Self::new_firesaber()),
            _ => Err(err!(errmsg!(
                "The key exchange scheme with local id {} is not recognised.", n,
            ), Invalid, Input)),
        }
    }
}

impl KeyExchangeScheme {

    pub const FIRESABER_PK_LEN:             usize = saber::FireSaber::PK_LEN;
    pub const FIRESABER_SK_LEN:             usize = saber::FireSaber::SK_LEN;
    pub const FIRESABER_SESSION_KEY_LEN:    usize = saber::FireSaber::SK_LEN;
    pub const FIRESABER_CIPHERTEXT_LEN:     usize = saber::FireSaber::CIPHERTEXT_BYTES;

    pub fn new_firesaber() -> Self {
        Self::FireSaber(Keys::randef())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enc_scheme_firesaber_00() -> Outcome<()> {
        let scheme = saber::FireSaber;
        // Bob generates keys at the server.
        let (bob_pk, bob_sk) = scheme.kem_keygen();
        // Bob sends the public key to Alice.
        // Alice generates her symmetric key using Bob's public key.
        let (alice_session_key, ciphertext) = scheme.kem_encap(&bob_pk);
        // Alice sends the scrambled symmetric key to Bob.
        // Bob recovers the symmetric key from Alice's scrambled transmissions.
        let bob_session_key = res!(scheme.kem_decap(
            &ciphertext.to_bytes::<{saber::FireSaber::CIPHERTEXT_BYTES}>(),
            &bob_sk,
        ));
        // Alice and Bob now encrypt/decrypt their session using the symmetric key.
        assert_eq!(alice_session_key, bob_session_key);
        msg!("Lengths:");
        msg!(" bob_pk: {}", bob_pk.byte_len());
        msg!(" bob_sk: {}", bob_sk.byte_len());
        msg!(" session_key: {}", alice_session_key.len());
        msg!(" ciphertext: {}", ciphertext.byte_len());
        Ok(())
    }
}
