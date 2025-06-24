use crate::keys::KeyManager;

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
};
use oxedyne_fe2o3_namex::id::{
    LocalId,
    InNamex,
    NamexId,
};

pub trait Signer:
    KeyManager
    + Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    /// Return a detached signature for the given message.
    fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>>;
    /// Verify the validity of the given detached signature for the given message. 
    fn verify(&self, msg: &[u8], sig: &[u8]) -> Outcome<bool>;
}

#[derive(Clone, Debug, Default)]
pub struct SignerDefAlt<
    D: Signer,
    G: Signer,
>(
    pub DefAlt<D, G>,
);

impl<
    D: Signer,
    G: Signer,
>
    std::ops::Deref for SignerDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: Signer,
    G: Signer,
>
    From<Option<G>> for SignerDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: Signer,
    G: Signer,
>
    From<Alt<G>> for SignerDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: Signer,
    G: Signer,
>
    InNamex for SignerDefAlt<D, G>
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
    G: Signer,
    D: Signer,
>
    Signer for SignerDefAlt<D, G>
{
    fn sign(&self, msg: &[u8]) -> Outcome<Vec<u8>> {
        match &self.0 {
            DefAlt::Default(inner) => inner.sign(msg),
            DefAlt::Given(inner) => inner.sign(msg),
            DefAlt::None => Err(err!(
                "Can't sign, signature not specified.";
            Configuration, Missing)),
        }
    }

    fn verify(&self, msg: &[u8], sig: &[u8]) -> Outcome<bool> {
        match &self.0 {
            DefAlt::Default(inner) => inner.verify(msg, sig),
            DefAlt::Given(inner) => inner.verify(msg, sig),
            DefAlt::None => Err(err!(
                "Can't verify, signature not specified.";
            Configuration, Missing)),
        }
    }

}

impl<
    G: Signer,
    D: Signer,
>
    KeyManager for SignerDefAlt<D, G>
{
    fn clone_with_keys(&self, pk: Option<&[u8]>, sk: Option<&[u8]>) -> Outcome<Self> {
        Ok(match &self.0 {
            DefAlt::Default(inner) => Self(
                DefAlt::Default(res!(inner.clone_with_keys(pk, sk))),
            ),
            DefAlt::Given(inner) => Self(
                DefAlt::Given(res!(inner.clone_with_keys(pk, sk))),
            ),
            DefAlt::None => Self(
                DefAlt::None, // TODO should this be an error?
            ),
        })
    }

    fn get_public_key(&self) -> Outcome<Option<&[u8]>> {
        match &self.0 {
            DefAlt::Default(inner) => inner.get_public_key(),
            DefAlt::Given(inner) => inner.get_public_key(),
            DefAlt::None => Err(err!(
                "Can't get public key, signature not specified.";
            Configuration, Missing)),
        }
    }

    fn get_secret_key(&self) -> Outcome<Option<&[u8]>> {
        match &self.0 {
            DefAlt::Default(inner) => inner.get_secret_key(),
            DefAlt::Given(inner) => inner.get_secret_key(),
            DefAlt::None => Err(err!(
                "Can't get secret key, signature not specified.";
            Configuration, Missing)),
        }
    }

    fn set_public_key(self, pk: Option<&[u8]>) -> Outcome<Self> {
        match self.0 {
            DefAlt::Default(inner) => Ok(Self(
                DefAlt::Default(res!(inner.set_public_key(pk))),
            )),
            DefAlt::Given(inner) => Ok(Self(
                DefAlt::Given(res!(inner.set_public_key(pk))),
            )),
            DefAlt::None => Err(err!(
                "Can't set public key, signature not specified.";
            Configuration, Missing)),
        }
    }

    fn set_secret_key(self, sk: Option<&[u8]>) -> Outcome<Self> {
        match self.0 {
            DefAlt::Default(inner) => Ok(Self(
                DefAlt::Default(res!(inner.set_secret_key(sk))),
            )),
            DefAlt::Given(inner) => Ok(Self(
                DefAlt::Given(res!(inner.set_secret_key(sk))),
            )),
            DefAlt::None => Err(err!(
                "Can't set secret key, signature not specified.";
            Configuration, Missing)),
        }
    }
}

impl<
    D: Signer,
    G: Signer,
>
    SignerDefAlt<D, G> {

    /// Use the given `Alt` `Signer` to override the `DefAlt` for encryption, if it is
    /// specified.  If not, use the `DefAlt` `Signer`.  This gives the user access to up to
    /// three different types of `Signer`.
    pub fn or_sign<OR: Signer>(&self, msg: &[u8], alt: &Alt<OR>) -> Outcome<Vec<u8>> {
        match alt {
            Alt::Specific(Some(inner)) => inner.sign(msg),  // Type OR Signer
            Alt::Specific(None) => Err(err!(
                "Can't sign, signature not specified.";
            Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.sign(msg),  // Type D Signer
                DefAlt::Given(inner) => inner.sign(msg),    // Type G Signer
                DefAlt::None => Err(err!(
                    "Can't sign, signature not specified.";
                Configuration, Missing)),
            },
        }
    }

    /// Use the given `Alt` `Signer` to override the `DefAlt` for decryption, if it is
    /// specified.  If not, use the `DefAlt` `Signer`.  This gives the user access to up to
    /// three different types of `Signer`.
    pub fn or_verify<OR: Signer>(&self, msg: &[u8], sig: &[u8], alt: &Alt<OR>) -> Outcome<bool> {
        match alt {
            Alt::Specific(Some(inner)) => inner.verify(msg, sig),   // Type OR Signer
            Alt::Specific(None) => Err(err!(
                "Can't verify, signature not specified.";
            Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.verify(msg, sig),   // Type D Signer
                DefAlt::Given(inner) => inner.verify(msg, sig),     // Type G Signer
                DefAlt::None => Err(err!(
                    "Can't verify, signature not specified.";
                Configuration, Missing)),
            },
        }
    }
}
