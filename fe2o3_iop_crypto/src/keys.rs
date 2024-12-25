use oxedize_fe2o3_core::prelude::*;

pub trait KeyManager {

    fn clone_with_keys(
        &self,
        pk: Option<&[u8]>,
        sk: Option<&[u8]>,
    )
        -> Outcome<Self> where Self: Sized;

    /// Get the public key.
    fn get_public_key(&self) -> Outcome<Option<&[u8]>>;
    /// Get the private key.
    fn get_secret_key(&self) -> Outcome<Option<&[u8]>>;
    /// Set the optional public key.
    fn set_public_key(self, pk: Option<&[u8]>) -> Outcome<Self> where Self: Sized;
    /// Set the optional secret key.
    fn set_secret_key(self, sk: Option<&[u8]>) -> Outcome<Self> where Self: Sized;
}

impl KeyManager for () {
    fn clone_with_keys(
        &self,
        _pk: Option<&[u8]>,
        _sk: Option<&[u8]>,
    )
        -> Outcome<Self>
    {
        Ok(())
    }

    fn get_public_key(&self) -> Outcome<Option<&[u8]>> { Ok(None) }
    fn get_secret_key(&self) -> Outcome<Option<&[u8]>> { Ok(None) }
    fn set_public_key(self, _pk: Option<&[u8]>) -> Outcome<Self> { Ok(()) }
    fn set_secret_key(self, _sk: Option<&[u8]>) -> Outcome<Self> { Ok(()) }
}
