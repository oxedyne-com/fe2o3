use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
};
use oxedyne_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

/// A Key Exchange Mechanism (KEM) involves the following steps:
/// 1. Bob generates a public, private key pair.
/// 2. Bob sends the public key to Alice.
/// 3. Alice uses `KeyExchanger::encap` to generate a secret session key, and an encrypted version.
/// 4. Alice sends the encrypted session key to Bob.
/// 5. Bob uses `KeyExchanger::decap` using his private key to recover the session key.
/// 6. Alice and Bob can now use the secret session key to encrypt their communications.
pub trait KeyExchanger:
    Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    /// Generate and encapsulate (encrypt) the session key.
    fn encap<
        const PK_LEN: usize,
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        pk: [u8; PK_LEN],
    )
        -> Outcome<(
            [u8; SESSION_KEY_LEN],
            [u8; CIPHERTEXT_LEN],
        )>;
    /// De-encapsulate ("decapsulate" or decrypt) the session key.
    fn decap<
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        ciphertext: [u8; CIPHERTEXT_LEN],
    )
        -> Outcome<
            [u8; SESSION_KEY_LEN],
        >;
}

#[derive(Clone, Debug, Default)]
pub struct KeyExchangerDefAlt<
    D: KeyExchanger,
    G: KeyExchanger,
>(
    pub DefAlt<D, G>,
);

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    std::ops::Deref for KeyExchangerDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    From<Option<G>> for KeyExchangerDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    From<Alt<G>> for KeyExchangerDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    InNamex for KeyExchangerDefAlt<D, G>
{
    fn name_id(&self) -> Outcome<NamexId> {
        match &self.0 {
            DefAlt::Default(inner) => inner.name_id(),
            DefAlt::Given(inner) => inner.name_id(),
            DefAlt::None => Err(err!(
                "No Namex id can be specified for DefAlt::None.";
            Missing, Bug)),
        }
    }

    fn local_id(&self) -> LocalId {
        match &self.0 {
            DefAlt::Default(inner)  => inner.local_id(),
            DefAlt::Given(inner)    => inner.local_id(),
            DefAlt::None            => LocalId::default(),
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
        match res!(D::assoc_names_base64(gname)) {
            Some(mut vd) => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => {
                    vd.extend(vg);
                    Ok(Some(vd))
                },
                None => Ok(Some(vd)),
            },
            None => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => Ok(Some(vg)),
                None => Ok(None),
            },
        }
    }
}

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    KeyExchanger for KeyExchangerDefAlt<D, G>
{
    fn encap<
        const PK_LEN: usize,
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        pk: [u8; PK_LEN],
    )
        -> Outcome<(
            [u8; SESSION_KEY_LEN],
            [u8; CIPHERTEXT_LEN],
        )>
    {
        match &self.0 {
            DefAlt::Default(inner) => inner.encap(pk),
            DefAlt::Given(inner) => inner.encap(pk),
            DefAlt::None => Err(err!(
                "Can't encapsulate, key exchanger not specified.";
            Configuration, Missing)),
        }
    }

    fn decap<
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
    >(
        &self,
        ciphertext: [u8; CIPHERTEXT_LEN],
    )
        -> Outcome<
            [u8; SESSION_KEY_LEN],
        >
    {
        match &self.0 {
            DefAlt::Default(inner) => inner.decap(ciphertext),
            DefAlt::Given(inner) => inner.decap(ciphertext),
            DefAlt::None => Err(err!(
                "Can't de-encapsulate, key exchanger not specified.";
            Configuration, Missing)),
        }
    }
}

impl<
    D: KeyExchanger,
    G: KeyExchanger,
>
    KeyExchangerDefAlt<D, G>
{
    /// Use the given `Alt` `KeyExchanger` to override the `DefAlt` for encryption, if it is
    /// specified.  If not, use the `DefAlt` `KeyExchanger`.  This gives the user access to up to
    /// three different types of `KeyExchanger`.
    pub fn or_encap<
        const PK_LEN: usize,
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
        OR: KeyExchanger,
    >(
        &self,
        pk: [u8; PK_LEN],
        alt: &Alt<OR>,
    )
        -> Outcome<(
            [u8; SESSION_KEY_LEN],
            [u8; CIPHERTEXT_LEN],
        )>
    {
        match alt {
            Alt::Specific(Some(inner)) => inner.encap(pk),      // Type OR KeyExchanger
            Alt::Specific(None) => Err(err!(
                "Can't encapsulate, key exchanger not specified.";
            Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.encap(pk),  // Type D KeyExchanger
                DefAlt::Given(inner) => inner.encap(pk),    // Type G KeyExchanger
                DefAlt::None => Err(err!(
                    "Can't encapsulate, key exchanger not specified.";
                Configuration, Missing)),
            },
        }
    }

    /// Use the given `Alt` `KeyExchanger` to override the `DefAlt` for decryption, if it is
    /// specified.  If not, use the `DefAlt` `KeyExchanger`.  This gives the user access to up to
    /// three different types of `KeyExchanger`.
    pub fn or_decap<
        const SESSION_KEY_LEN: usize,
        const CIPHERTEXT_LEN: usize,
        OR: KeyExchanger,
    >(
        &self,
        ciphertext: [u8; CIPHERTEXT_LEN],
        alt: &Alt<OR>,
    )
        -> Outcome<
            [u8; SESSION_KEY_LEN],
        >
    {
        match alt {
            Alt::Specific(Some(inner)) => inner.decap(ciphertext),      // Type OR KeyExchanger
            Alt::Specific(None) => Err(err!(
                "Can't de-encapsulate, key-exchanger not specified.";
            Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.decap(ciphertext),  // Type D KeyExchanger
                DefAlt::Given(inner) => inner.decap(ciphertext),    // Type G KeyExchanger
                DefAlt::None => Err(err!(
                    "Can't de-encapsulate, key-exchanger not specified.";
                Configuration, Missing)),
            },
        }
    }
}
