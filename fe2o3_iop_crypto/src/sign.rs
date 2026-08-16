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

/// One signature to check, in the form a batch wants it.
///
/// Each item carries its own public key, because a batch is not a batch of one
/// signer: a version control history is signed by everyone who has written to
/// it, and the point of checking many at once is lost if they must first be
/// sorted by author.
#[derive(Clone, Copy, Debug)]
pub struct BatchItem<'a> {
    /// The public key of whoever signed, as the scheme encodes it.
    pub public: &'a [u8],
    /// The bytes that were signed.
    pub msg:    &'a [u8],
    /// The detached signature over `msg`.
    pub sig:    &'a [u8],
}

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

    /// Verify many signatures, each against the public key its item carries,
    /// and report whether every one of them holds.
    ///
    /// This is an optimisation and nothing more. A scheme that has a batch
    /// verification equation may check the whole set for far less than the sum
    /// of the parts; the default here simply checks them one at a time, so an
    /// implementation that has nothing better to offer need do nothing.
    ///
    /// # What a `false` does and does not tell the caller
    ///
    /// It says that the set does not hold. It does not say which member of it
    /// failed, and a scheme verifying the set as a whole cannot say. A caller
    /// that must name the culprit -- and one refusing a history should -- falls
    /// back to [`Signer::verify`] over the items to find it. The same goes for
    /// an error: a batch that could not be attempted says so without saying
    /// which item could not be attempted.
    ///
    /// # An empty batch
    ///
    /// Holds, vacuously. There is nothing in it that does not verify.
    fn verify_batch(&self, items: &[BatchItem<'_>])
        -> Outcome<bool>
        where Self: Sized
    {
        verify_each(self, items)
    }
}

/// Verifies the items one at a time, stopping at the first that does not hold.
///
/// The body of [`Signer::verify_batch`]'s default, exposed so that an
/// implementation which is faster for some of its schemes and not for others
/// can hand the rest here rather than writing the loop again.
pub fn verify_each<S: Signer>(scheme: &S, items: &[BatchItem<'_>])
    -> Outcome<bool>
{
    for item in items {
        let bound = res!(scheme.clone_with_keys(Some(item.public), None));
        if !res!(bound.verify(item.msg, item.sig)) {
            return Ok(false);
        }
    }
    Ok(true)
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

    /// Hands the batch to whichever scheme is in force, rather than taking the
    /// default, so that an inner scheme's batch equation is not lost behind the
    /// wrapper.
    fn verify_batch(&self, items: &[BatchItem<'_>])
        -> Outcome<bool>
        where Self: Sized
    {
        match &self.0 {
            DefAlt::Default(inner) => inner.verify_batch(items),
            DefAlt::Given(inner) => inner.verify_batch(items),
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
