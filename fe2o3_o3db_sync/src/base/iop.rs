use crate::{
    O3db,
    prelude::*,
    base::constant,
    comm::msg::OzoneMsg,
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::{
    Meta,
    Database,
    RestSchemesOverride,
};
use oxedize_fe2o3_iop_hash::{
    api::Hasher,
    csum::Checksummer,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedize_fe2o3_namex::id::{
    InNamex,
    NamexId,
};

use std::{
    sync::{
        Arc,
        RwLock,
    },
};


impl<
    const UIDL: usize,                  // User identifier byte length.
    UID:    NumIdDat<UIDL> + 'static,   // User identifier.            
    ENC:    Encrypter + 'static,        // Symmetric encryption of data at rest.
    KH:     Hasher + 'static,           // Hashes database keys.
	PR:     Hasher + 'static,           // Pseudo-randomiser hash to distribute cache data.
    CS:     Checksummer + 'static,      // Checks integrity of data at rest.
>
    Database<UIDL, UID, ENC, KH> for O3db<UIDL, UID, ENC, KH, PR, CS>
{
    fn insert(
        &self,
        key:    Dat,
        val:    Dat,
        user:   UID,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(bool, usize)>
    {
        let resp = res!(self.api().put(
            key,
            val,
            user,
            or,
        ));
        let num_chunks = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::Chunks(n) => n,
            msg => return Err(err!(
                "Expected an OzoneMsg::Chunks message, received a: {:?}", msg;
            Bug, Unexpected)),
        };
        if num_chunks == 1 {
            match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
                OzoneMsg::KeyExists(exists) => return Ok((exists, 1)),
                msg => return Err(err!(
                    "Expected an OzoneMsg::KeyExists message, received a: {:?}", msg;
                Bug, Unexpected)),
            }
        } else {
            let (_, msgs) = res!(resp.recv_number(num_chunks, constant::USER_REQUEST_WAIT));
            let mut exists = false;
            for msg in msgs {
                match msg {
                    OzoneMsg::KeyChunkExists(b, 0) => exists = b,
                    _ => (),
                }
            }
            return Ok((exists, num_chunks));
        }
    }

    fn get(
        &self,
        key:    &Dat,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<(Dat, Meta<UIDL, UID>)>>
    {
        self.api().get_wait(
            key,
            or,
        )
    }

    fn delete(
        &self,
        key:    &Dat,
        user:   UID,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<bool>
    {
        let resp = self.api().responder();
        res!(self.api().delete_using_responder(
            key,
            user,
            or,
            resp.clone(),
        ));
        match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::KeyExists(b) => Ok(b),
            msg => Err(err!(
                "Expected an OzoneMsg::KeyExists message, received a: {:?}", msg;
            Bug, Unexpected)),
        }
    }
}

/// `LocalOzoneApi` is unfortunately necessary to satisfy the compiler regarding E0210. 
#[derive(Debug)]
struct LocalOzoneApi<
    const UIDL: usize,
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
    PR: Hasher + 'static,
    CS: Checksummer + 'static,
>(Arc<RwLock<OzoneApi<UIDL, UID, ENC, KH, PR, CS>>>);

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
    PR: Hasher + 'static,
    CS: Checksummer + 'static,
>
    InNamex for LocalOzoneApi<UIDL, UID, ENC, KH, PR, CS>
{
    fn name_id(&self) -> Outcome<NamexId> {
        let unlocked_api = lock_read!(self.0);
        unlocked_api.name_id()
    }
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
    PR: Hasher + 'static,
    CS: Checksummer + 'static,
>
    std::ops::Deref for LocalOzoneApi<UIDL, UID, ENC, KH, PR, CS>
{
    type Target = Arc<RwLock<OzoneApi<UIDL, UID, ENC, KH, PR, CS>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<
    const UIDL: usize,                  // User identifier byte length.
    UID:    NumIdDat<UIDL> + 'static,   // User identifier.            
    ENC:    Encrypter + 'static,        // Symmetric encryption of data at rest.
    KH:     Hasher + 'static,           // Hashes database keys.
	PR:     Hasher + 'static,           // Pseudo-randomiser hash to distribute cache data.
    CS:     Checksummer + 'static,      // Checks integrity of data at rest.
>
    Database<UIDL, UID, ENC, KH> for LocalOzoneApi<UIDL, UID, ENC, KH, PR, CS>
{
    fn insert(
        &self,
        key:    Dat,
        val:    Dat,
        user:   UID,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<(bool, usize)>
    {
        let unlocked_api = lock_read!(self.0);
        let resp = res!(unlocked_api.put(
            key,
            val,
            user,
            or,
        ));
        let num_chunks = match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::Chunks(n) => n,
            msg => return Err(err!(
                "Expected an OzoneMsg::Chunks message, received a: {:?}", msg;
            Bug, Unexpected)),
        };
        if num_chunks == 1 {
            match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
                OzoneMsg::KeyExists(exists) => return Ok((exists, 1)),
                msg => return Err(err!(
                    "Expected an OzoneMsg::KeyExists message, received a: {:?}", msg;
                Bug, Unexpected)),
            }
        } else {
            let (_, msgs) = res!(resp.recv_number(num_chunks, constant::USER_REQUEST_WAIT));
            let mut exists = false;
            for msg in msgs {
                match msg {
                    OzoneMsg::KeyChunkExists(b, 0) => exists = b,
                    _ => (),
                }
            }
            return Ok((exists, num_chunks));
        }
    }

    fn get(
        &self,
        key:    &Dat,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<Option<(Dat, Meta<UIDL, UID>)>>
    {
        let unlocked_api = lock_read!(self.0);
        unlocked_api.get_wait(
            key,
            or,
        )
    }

    fn delete(
        &self,
        key:    &Dat,
        user:   UID,
        or:     Option<&RestSchemesOverride<ENC, KH>>,
    )
        -> Outcome<bool>
    {
        let unlocked_api = lock_read!(self.0);
        let resp = unlocked_api.responder();
        res!(unlocked_api.delete_using_responder(
            key,
            user,
            or,
            resp.clone(),
        ));
        match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::KeyExists(b) => Ok(b),
            msg => Err(err!(
                "Expected an OzoneMsg::KeyExists message, received a: {:?}", msg;
            Bug, Unexpected)),
        }
    }
}
