use crate::{
    base::cfg::OzoneConfig,
    comm::response::Wait,
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::Encoding,
};
use oxedize_fe2o3_jdat::{
    usr::UsrKindCode,
    version::SemVer,
};

use std::time::Duration;

impl OzoneConfig {
    pub fn check_constants() -> Outcome<()> {
        if CHECK_INTERVAL > USER_REQUEST_TIMEOUT {
            return Err(err!(
                "The prescribed constant check interval, {:?}, should not be larger than the \
                constant maximum wait, {:?}.", CHECK_INTERVAL,
                USER_REQUEST_TIMEOUT;
            Invalid, Input));
        }
        Ok(())
    }
}

pub const VERSION:                      SemVer = SemVer::new(0, 5, 0);
pub const NAMEX_ID:                    &'static str = "QByizewdCnRH/E6ksx4rOnqv5lFuB6PX1EA4Z5kNQwA=";

// Files.
pub const MAX_ZONES:                    u16 = 100;
pub const DEFAULT_MAX_ZONE_DIR_BYTES:   u64 = 104_857_600; // 100 MiB
pub const CONFIG_FILENAME:              &'static str = "config.jdat";
pub const DB_UID_CHAR_LEN:              usize = 5;

pub const DATA_FILE_EXT:                &'static str = "dat";
pub const INDEX_FILE_EXT:               &'static str = "ind";
pub const DB_DIR_PREFIX:                &'static str = "o3db";
pub const HASH_BYTES:                   usize = 8; // u64
pub const CACHE_HASH_BYTES:             usize = 4; // u32

// File size management.
// When the total length of old values in a data file exceeds the following percentage of the
// maximum data file length, garbage collection on the file is triggered.
pub const OLD_DATA_PERCENT_GC_TRIGGER:  f64 = 30.0;

// File reading cache.
pub const FILE_CACHE_EXPIRY_SECS:       u64 = 15*60; // 15 mins
pub const MAX_CACHED_FILES:             usize = 200;

// Resource management.
pub const CACHE_JETTISON_FRAC_OF_LIM:   f64 = 0.20;

// Bots.
pub const BOT_ERR_COUNT_WARNING:        usize = 10;
pub const STACK_SIZE:                   usize = 2 * 1024 * 1024;

// Shutdown.
// Wait for all bots to idle after shutting off server.
pub const SHUTDOWN_MAX_WAIT:            Duration = Duration::from_secs(3);

// Intervals.
pub const HEALTH_CHECK_INTERVAL:        Duration = Duration::from_secs(60);

// Busy waiting intervals.
pub const CHECK_INTERVAL:                       Duration = Duration::from_millis(100);
pub const CONFIGWATCHER_CHECK_INTERVAL_SECS:    u64 = 3;
pub const SERVER_INT_CHANNEL_CHECK_INTERVAL:    Duration = Duration::from_nanos(1_000);
pub const SERVER_EXT_SOCKET_CHECK_INTERVAL:     Duration = Duration::from_nanos(999_000);

// Timeouts.
// User timeouts must last longer than internal bot timeouts to avoid lockups.
pub const USER_REQUEST_TIMEOUT:                 Duration =
    BOT_REQUEST_TIMEOUT.saturating_add(Duration::from_secs(1));
pub const USER_REQUEST_WAIT:                    Wait = Wait {
    max_wait:       USER_REQUEST_TIMEOUT,
    check_interval: CHECK_INTERVAL,
};
pub const BOT_REQUEST_TIMEOUT:                  Duration = Duration::from_secs(5);
pub const BOT_REQUEST_WAIT:                     Wait = Wait {
    max_wait:       BOT_REQUEST_TIMEOUT,
    check_interval: CHECK_INTERVAL,
};
pub const ZONE_STATE_UPDATER_LISTEN_TIMEOUT:    Duration = Duration::from_millis(300);
//pub const GET_DATA_WAIT:                        Wait = Wait::new_default();
pub const PING_TIMEOUT:                         Duration = Duration::from_secs(5);

// ConfigBot.
pub const CONFIGWATCHER_REFRESH_FILE_AFTER_N_CHECKS: usize = 100;

// ServerBot.
pub const SERVER_ADDRESS:                   &'static str = "127.0.0.1";
pub const UDP_BUFFER_SIZE:                  usize = 1_400;
// Mostly completely arbitrary...
pub const POW_CREATE_TIMEOUT:               Duration = Duration::from_secs(30);
pub const POW_CREATE_COUNT_LIM:             usize = usize::MAX;
pub const DEFAULT_UDP_PACKET_SIZE:          usize = 700;
pub const REQ_TIMER_LEN:                    usize = 100;
pub const MAX_ALLOWED_AVG_REQ_PER_SEC:      u64 = 30;
pub const POW_MAX_ZERO_BITS:                usize = 30;
pub const POW_NONCE_LEN:                    usize = 8;
pub const POW_CODE_LEN:                     usize = 8;
pub const POW_ADDR_LEN:                     usize = 16;
pub const POW_TIMESTAMP_LEN:                usize = 8;
pub const POW_PREFIX_LEN: usize = 
    POW_ADDR_LEN +
    POW_CODE_LEN;
pub const POW_PREIMAGE_LEN: usize = 
    POW_ADDR_LEN +
    POW_CODE_LEN +
    POW_TIMESTAMP_LEN;
pub const POW_INPUT_LEN: usize = 
    POW_PREIMAGE_LEN +
    POW_NONCE_LEN;
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
pub const DEFAULT_MSG_ENCODING:             Encoding = Encoding::Binary;

// Schemes =====================================================================
// Chunking.
// Min chunk size rationale:
// (Dat::BU64 + AES_GCM) overhead, safety factor of 2
pub const MIN_CHUNK_SIZE:               usize = (9 + 12)*2;
pub const DEFAULT_REST_CHUNK_BYTES:     usize = 1_024_000; // 1 MiB
pub const DEFAULT_WIRE_CHUNK_BYTES:     usize = 1_024; // 1 KiB

// Hashing.
pub const KEY_HASH_SALT:                [u8; 16] = SALT16;
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

// Encryption.
pub const DEFAULT_SYMMETRIC_KEY_BYTES: usize = 32;

// Randomness.
pub const DB_UID_CHARS: &'static str = 
    //          1         2         3         4         5
    // 123456789012345678901234567890123456789012345678901234
      "ABCDEFGHKMNPQRSTUVWXYZabcedefghkmnpqrstuvwxyz123456789";

// Document Data Abstraction Layer.
pub const USER_KIND_DIR_CODE:       UsrKindCode = 5;
pub const USER_KIND_DOC_CODE:       UsrKindCode = 6;
pub const DOC_PATH_LEN_LIMIT:       usize = 1024;

pub const USER_ID_BYTE_LEN:         usize = 16;

pub const MAX_FILE_BYTES: u64 = i64::MAX as u64;
pub const MAX_FILE_TO_CHUNKING_THRESHOLD_RATIO: f64 = 0.8;
pub const MAX_FILE_TO_CHUNK_SIZE_RATIO: f64 = 0.3;
/// When restarting an ozone database, the live file in a zone is usually chosen to be one with
/// the highest file number.  This will remain the case if the ratio of the live data file size 
/// to the maximum data file size is below this fraction, otherwise a new live file will be
/// created.
pub const LIVE_FILE_INIT_SIZE_RATIO_THRESHOLD: f64 = 0.5;

// Credits:
// Image: https://ascii-generator.site/
// Text: https://www.asciiart.eu/text-to-ascii-art Nancyj Improved with touch ups.
pub const SPLASH: &'static str =
r#"
   
     .:::::::::::::::::::::. 
   .'                       '.
   .:                       :.
   ..                       ..     .o88888o.                dP dP      
   ..                       ..    d8'     `8b               88 88      
   ..   ....         ....   ..    88       88         .d888b88 88d888b.
   ..  .....:       :.....  ..    88       88 d8888b. 88'  `88 88'  `88
   ..  '......     ......'  ..    Y8.     .8P     `88 88.  .88 88.  .88
   ..       ...:::...       ..     `888888P'   aaad8' `88888P8 88Y8888'
   ..        .......        ..                    `88
   ..         '...'         ..                d88888P
   .:                       :.
    '-:::::::::::::::::::::-'                            
                                                         Ozone Database
   
"#;


