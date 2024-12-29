use crate::srv::{
    cfg::ServerConfig,
    msg::external::MsgType,
    pow::DifficultyProfile,
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::Encoding,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};

use std::{
    mem,
    time::Duration,
};

impl ServerConfig {
    pub fn check_constants() -> Outcome<()> {
        //if CHECK_INTERVAL > USER_REQUEST_TIMEOUT {
        //    return Err(err!(
        //        "The prescribed constant check interval, {:?}, should not be larger than the \
        //        constant maximum wait, {:?}.", CHECK_INTERVAL,
        //        USER_REQUEST_TIMEOUT,
        //    ), Invalid, Input));
        //}
        Ok(())
    }
}
//
pub const VERSION:                          SemVer = SemVer::new(0, 5, 0);
pub const STACK_SIZE:                       usize = 2 * 1024 * 1024;
pub const UDP_BUFFER_SIZE:                  usize = 1_400;
//// Mostly completely arbitrary...
pub const POW_CREATE_TIMEOUT:               Duration = Duration::from_secs(30);
pub const POW_CREATE_COUNT_LIM:             usize = usize::MAX;
pub const POW_TIME_HORIZON_SEC:             u64 = 600;
pub const DEFAULT_UDP_PACKET_SIZE:          usize = 700;
pub const REQ_TIMER_LEN:                    usize = 100;
pub const MAX_ALLOWED_AVG_REQ_PER_SEC:      u64 = 30;
pub const POW_DIFFICULTY_PROFILE:           DifficultyProfile = DifficultyProfile::Linear;
pub const POW_MAX_ZERO_BITS:                u16 = 30;
pub const POW_MIN_ZERO_BITS:                u16 = 0; // TODO add bypass when zbits == 0
pub const POW_NONCE_LEN:                    usize = 8;
pub const POW_CODE_LEN:                     usize = 8;
pub const POW_ADDR_LEN:                     usize = 16;
pub const POW_TIMESTAMP_LEN:                usize = 8;
pub const POW_PREFIX_LEN: usize = 
    POW_ADDR_LEN
    + POW_CODE_LEN;
pub const POW_PREIMAGE_LEN: usize = 
    POW_ADDR_LEN
    + POW_CODE_LEN
    + POW_TIMESTAMP_LEN;
pub const POW_INPUT_LEN: usize = 
    POW_PREIMAGE_LEN
    + POW_NONCE_LEN;
pub const THROTTLED_INTERVAL_MIN:           Duration = Duration::from_secs(1);
pub const ADDR_THROTTLE_SUNSET_SECS_MIN:    u64 = 1_800; // 30 min
pub const ADDR_THROTTLE_SUNSET_SECS_MAX:    u64 = 259_200; // 3 days
pub const THROTTLE_COUNT_BEFORE_BLACKLIST:  u16 = 10;
pub const SESSION_REQUEST_EXPIRY:           Duration = Duration::from_secs(600); // 10 min
pub const PARTIAL_MESSAGE_SUNSET:           Duration = Duration::from_secs(600); // 10 min
pub const MSG_ASSEMBLY_SUNSET:              Duration = Duration::from_secs(600);
pub const MSG_ASSEMBLY_IDLE_MAX:            Duration = Duration::from_secs(60);
pub const MSG_ASSEMBLY_REP_TOTAL_LIM:       u8 = 128;
pub const MSG_ASSEMBLY_REP_PACKET_LIM:      u8 = 32;
pub const MSG_ASSEMBLY_GC_INTERVAL:         Duration = Duration::from_secs(600);
pub const MSG_ASSEMBLY_SHARDS:              usize = 10;
pub const DEFAULT_MSG_ENCODING:             Encoding = Encoding::Binary;
pub const UGRD_SHARDMAP_INIT_SHARDS:        usize = 10;
pub const AGRD_SHARDMAP_INIT_SHARDS:        usize = 10;
pub const GUARD_SHARDMAP_SALT_LEN:          usize = 8;

pub const SESSION_ID_KIND:                  Kind = Kind::U64;

pub const MSG_TYPE_BYTE_LEN:                usize = mem::size_of::<MsgType>();

pub const SERVER_INT_CHANNEL_CHECK_INTERVAL:    Duration = Duration::from_nanos(1_000);
pub const SERVER_EXT_SOCKET_CHECK_INTERVAL:     Duration = Duration::from_nanos(999_000);

pub const SERVER_BOT_ERROR_COUNT_WARNING:   usize = 10;

// Schemes =====================================================================
// Chunking.
// Min chunk size rationale:
// (Daticle::BU64 + AES_GCM) overhead, safety factor of 2
pub const MIN_CHUNK_SIZE:               usize = (9 + 12)*2;
//pub const DEFAULT_WIRE_CHUNK_BYTES:     usize = 1_024; // 1 KiB

// Hashing.
pub const USER_PASS_HASH_SALT_LEN: usize = 16;
pub const SALT8: [u8; 8] = [
    0x15, 0x04, 0x84, 0x1e, 0xf2, 0x07, 0x19, 0xbc,
];
pub const SALT16: [u8; 16] = [
    0xc9, 0x48, 0xcc, 0xbe, 0xd5, 0xd1, 0x16, 0x6c,
    0xc2, 0x2a, 0x78, 0x85, 0xce, 0x3e, 0x25, 0xcd,
];
pub const SALT32: [u8; 32] = [
    0xbd, 0xd5, 0x81, 0xba, 0x0c, 0xeb, 0x4f, 0xad,
    0x23, 0x72, 0xd4, 0xac, 0x92, 0xeb, 0xa6, 0x6f,
    0xf9, 0x65, 0x4f, 0x04, 0xca, 0xd5, 0x8b, 0x73,
    0xb2, 0xfa, 0x72, 0x39, 0x21, 0x6b, 0x6c, 0x5f,
];
