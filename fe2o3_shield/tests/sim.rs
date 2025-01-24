use oxedize_fe2o3_shield::{
    cfg::ServerConfig,
    core::Shield,
    guard::{
        data::UserData,
        user::{
            UserLog,
            UserState,
        },
    },
    msg::syntax,
};

use oxedize_fe2o3_bot::id::BotId;
use oxedize_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedize_fe2o3_crypto::{
    enc::EncryptionScheme,
    sign::SignatureScheme,
};
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedize_fe2o3_net::id::{
    MessageId,
    SessionId,
    UserId,
};

use std::{
    mem,
    sync::Arc,
    time::Duration,
};

use base64;

pub fn test_sim(_filter: &'static str) -> Outcome<()> {



    Ok(())
}
