use oxedize_fe2o3_shield::{
    cfg::ShieldConfig,
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

pub fn test_msg(_filter: &'static str) -> Outcome<()> {

    const POW_ZBITS: u16 = 5;

    pub type MidTyp = u64; // Message Id
    pub type SidTyp = u64; // Session Id
    pub type UidTyp = u128; // User Id
    
    const MID_LEN: usize = mem::size_of::<MidTyp>();
    const SID_LEN: usize = mem::size_of::<SidTyp>();
    const UID_LEN: usize = mem::size_of::<UidTyp>();
    
    pub type Mid = MessageId<{MID_LEN}, MidTyp>;
    pub type Sid = SessionId<{SID_LEN}, SidTyp>;
    pub type Uid = UserId<{UID_LEN}, UidTyp>;

    let server_id = BotId(Rand::rand_u64());
    let chunk_cfg = ShieldConfig::new_chunk_cfg(1_000, 200, true, true);
    let syntax = res!(ShieldConfig::syntax_default());
    let shield_params = Shield::params(
        [0u8; 8],
        Mid::default(),
        Sid::default(),
        Uid::default(),
        server_id,
        None::<EncryptionScheme>,
        None::<ChecksumScheme>,
        None::<HashScheme>,
        Some(SignatureScheme::new_ed25519()),
        None::<EncryptionScheme>,
        Some(chunk_cfg),
        syntax.clone(),
    );
    let (shield, server) = res!(Shield::new(
        None::<&str>,
        shield_params,
    ));

    // Add a user.
    const TEST_USER: Uid = UserId(1);
    const TEST_USER_SECRET_POW_CODE: [u8; 8] = [0x03, 0x48, 0x9f, 0x6a, 0xd4, 0x86, 0xe3, 0x35];
    // The code can also be distributed to the user alphanumerically.
    test!("Code: {:02x?}", TEST_USER_SECRET_POW_CODE);
    let code_string_std_no_pad = base64::encode_config(TEST_USER_SECRET_POW_CODE, base64::STANDARD_NO_PAD);
    test!("Code [std_no_pad]:       '{}'", code_string_std_no_pad);
    let code_from_std_no_pad = base64::decode_config(code_string_std_no_pad, base64::STANDARD_NO_PAD);
    test!("Code [std_no_pad]:       {:02x?}", code_from_std_no_pad);

    let (key, locked_map) = res!(shield.user_guard.get_locked_map(&TEST_USER));
    {
        let mut unlocked_map = lock_write!(locked_map);
        let ulog = UserLog {
            state: UserState::Whitelist,
            data: UserData {
                code: Some(TEST_USER_SECRET_POW_CODE),
                ..Default::default()
            },
        };
        unlocked_map.insert(key, ulog);
    }

    res!(shield.start(server));

    // Now send messages to the server.
    {

        let msg_builder = res!(shield.msg_builder_default(
            TEST_USER_SECRET_POW_CODE,
            POW_ZBITS,
        ));

        res!(syntax::HReq1::send(
            syntax.clone(),
            &msg_builder,
            None::<Mid>,
            None::<Sid>,
            Uid::new(1),
        ));

        thread::sleep(Duration::from_secs(3));

        for line in oxedize_fe2o3_text::string::to_lines(fmt!("{:?}", shield.user_guard), "  ") {
            debug!("{}", line);
        }

    }

    Ok(())
}
