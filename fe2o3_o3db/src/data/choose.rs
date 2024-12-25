use crate::{
    prelude::*,
    base::index::{
        BotPoolInd,
        WorkerInd,
        ZoneInd,
    },
};

use oxedize_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

// Just a name space for some associated functions.
pub struct ChooseCache<PR: Hasher>(std::marker::PhantomData<PR>);

impl<PR: Hasher> ChooseCache<PR> {

    pub fn choose_cbot(
        hform:  &HashForm,
        nz:     u16,
        nc:     u16,
    )
        -> Outcome<(WorkerInd, alias::ChooseHash)>
    {
        let (n, chash) = res!(Self::choose_cbot_prepare(hform));
        let bot = Self::choose_cbot_select(n, nz, nc);
        Ok((bot, chash))
    }

    /// Convert the `HashForm` to a consistent unsigned integer `alias::ChooseHashUint` and its
    /// byte representation.
    pub fn choose_cbot_prepare(
        hform:  &HashForm,
    )
        -> Outcome<(alias::ChooseHashUint, alias::ChooseHash)>
    {
        match hform {
            HashForm::Identity(h) | HashForm::Bytes(h) => {
                if h.len() >= constant::CACHE_HASH_BYTES {
                    let byts = res!(<alias::ChooseHash>::try_from(
                        &h[..constant::CACHE_HASH_BYTES]));
                    Ok((alias::ChooseHashUint::from_be_bytes(byts), byts))
                } else {
                    return Err(err!(errmsg!(
                        "The hash length of {} does not meet the minimum requirement of {}.",
                        h.len(), constant::CACHE_HASH_BYTES,
                    ), Input, Invalid, Bug));
                }
            },
            HashForm::Bytes32(a32) => {
                let byts = res!(<alias::ChooseHash>::try_from(
                    &a32[..constant::CACHE_HASH_BYTES]));
                Ok((alias::ChooseHashUint::from_be_bytes(byts), byts))
            },
            HashForm::U128(nu128) => {
                let n = *nu128 as alias::ChooseHashUint;
                Ok((n, n.to_be_bytes()))
            },
            HashForm::U64(nu64) => {
                let n = *nu64 as alias::ChooseHashUint;
                Ok((n, n.to_be_bytes()))
            },
            HashForm::U32(nu32) => {
                let n = *nu32 as alias::ChooseHashUint;
                Ok((n, n.to_be_bytes()))
            },
        }
    }

    pub fn choose_cbot_select(
        n:  alias::ChooseHashUint,
        nz: u16,
        nc: u16,
    )
        -> WorkerInd
    {
        let n1 = n as u16;
        let n2 = (n >> 16) as u16;
        WorkerInd::new(
            ZoneInd::new(n1 % nz),
            BotPoolInd::new(n2 % nc),
        )
    }

}
