use crate::{
    constant,
    id::Uid,
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedize_fe2o3_crypto::{
    keys::KeyPair,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    fmt::Debug,
    marker::PhantomData,
    ops::Deref,
};

pub struct UserKeys<
    UID: NumIdDat<{constant::UID_LEN}>,
    M: MapMut<Uid<UID>, KeyPair> + Clone + Debug + Default,
>(
    pub M,
    PhantomData<UID>,
);

impl<
    UID: NumIdDat<{constant::UID_LEN}>,
    M: MapMut<Uid<UID>, KeyPair> + Clone + Debug + Default,
>
    Deref for UserKeys<UID, M>
{
    type Target = M;
    fn deref(&self) -> &Self::Target { &self.0 }
}

