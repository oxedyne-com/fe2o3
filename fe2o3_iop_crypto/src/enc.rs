use crate::keys::KeyManager;

use oxedize_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
        Override,
    },
};
use oxedize_fe2o3_namex::id::{
    LocalId,
    InNamex,
    NamexId,
};

pub trait Encrypter:
    KeyManager
    + Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    /// Encrypt the given data to a new vector, possibly with included metadata.
    fn encrypt(&self, data: &[u8]) -> Outcome<Vec<u8>>;
    /// Decrypt the given data to a new vector, possibly requiring the presence of included metadata.
    fn decrypt(&self, data: &[u8]) -> Outcome<Vec<u8>>;
    fn is_identity(&self) -> bool;
}

impl Encrypter for () {
    fn encrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> { Ok(data.to_vec()) }
    fn decrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> { Ok(data.to_vec()) }
    fn is_identity(&self) -> bool { true }
}

#[derive(Clone, Debug, Default)]
pub struct EncrypterDefAlt<
    D: Encrypter,
    G: Encrypter,
>(
    pub DefAlt<D, G>,
);

impl<
    D: Encrypter,
    G: Encrypter,
>
    std::ops::Deref for EncrypterDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: Encrypter,
    G: Encrypter,
>
    From<Option<G>> for EncrypterDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: Encrypter,
    G: Encrypter,
>
    From<Alt<G>> for EncrypterDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: Encrypter,
    G: Encrypter,
>
    InNamex for EncrypterDefAlt<D, G>
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
    D: Encrypter,
    G: Encrypter,
>
    Encrypter for EncrypterDefAlt<D, G>
{

    fn encrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> {
        match &self.0 {
            DefAlt::Default(inner) => inner.encrypt(data),
            DefAlt::Given(inner) => inner.encrypt(data),
            DefAlt::None => Err(err!(
                "Can't encrypt, encrypter not specified.";
            Configuration, Missing)),
        }
    }

    fn decrypt(&self, data: &[u8]) -> Outcome<Vec<u8>> {
        match &self.0 {
            DefAlt::Default(inner) => inner.decrypt(data),
            DefAlt::Given(inner) => inner.decrypt(data),
            DefAlt::None => Err(err!(
                "Can't decrypt, encrypter not specified.";
            Configuration, Missing)),
        }
    }

    fn is_identity(&self) -> bool {
        match &self.0 {
            DefAlt::Default(inner) => inner.is_identity(),
            DefAlt::Given(inner) => inner.is_identity(),
            DefAlt::None => true,
        }
    }
}

impl<
    G: Encrypter,
    D: Encrypter,
>
    KeyManager for EncrypterDefAlt<D, G>
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
    D: Encrypter,
    G: Encrypter,
>
    EncrypterDefAlt<D, G>
{
    /// Possibly override the hasher in `EncrypterDefAlt`.
    pub fn or_encrypt(
        &self,
        data:   &[u8],
        or:     Option<&Override<D, G>>,
    )
        -> Outcome<Vec<u8>>
    {
        match or {
            None | Some(Override::PassThrough)  => self.encrypt(data),
            Some(Override::Default(inner))      => inner.encrypt(data),
            Some(Override::Given(inner))        => inner.encrypt(data),
            Some(Override::None)                => ().encrypt(data),
        }
    }

    pub fn or_decrypt(
        &self,
        data:   &[u8],
        or:     Option<&Override<D, G>>,
    )
        -> Outcome<Vec<u8>>
    {
        match or {
            None | Some(Override::PassThrough)  => self.decrypt(data),
            Some(Override::Default(inner))      => inner.decrypt(data),
            Some(Override::Given(inner))        => inner.decrypt(data),
            Some(Override::None)                => ().decrypt(data),
        }
    }

    pub fn or_is_identity(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> bool
    {
        match or {
            None | Some(Override::PassThrough)  => self.is_identity(),
            Some(Override::Default(inner))      => inner.is_identity(),
            Some(Override::Given(inner))        => inner.is_identity(),
            Some(Override::None)                => ().is_identity(),
        }
    }

    pub fn clone_from_or(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> Self
    {
        match or {
            None | Some(Override::PassThrough)  => self.clone(),
            Some(Override::Default(inner))      => Self(DefAlt::Default(inner.clone())),
            Some(Override::Given(inner))        => Self(DefAlt::Given(inner.clone())),
            Some(Override::None)                => Self(DefAlt::None),
        }
    }

    pub fn or_debug(
        &self,
        or: Option<&Override<D, G>>,
    )
        -> String
    {
        match or {
            None | Some(Override::PassThrough) => fmt!("{:?}", self),
            _ => fmt!("{:?}", or),
        }
    }
}
