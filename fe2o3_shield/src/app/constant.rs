use oxedyne_fe2o3_jdat::version::SemVer;
use oxedyne_fe2o3_o3db_sync::{
    comm::response::Wait,
};

pub const VERSION:                          SemVer = SemVer::new(0, 1, 0);
pub const DB_DIR:                           &'static str = "o3db";
pub const DEFAULT_LOG_LEVEL:                &'static str = "info";
pub const LOG_FILE_EXTENSION:               &'static str = "log";

pub const CONFIG_NAME:                      &'static str = "config.jdat";
pub const WALLET_NAME:                      &'static str = "wallet.jdat";
// Retain enough passhashes to be satisfied that the user is not reusing recent passphrases.
pub const NUM_PREV_PASSHASHES_TO_RETAIN:    usize = 10;
// Timeouts.
pub const GET_DATA_WAIT:                    Wait = Wait::new_default();
// Key derivation functions.
pub const KDF_HASH_LEN:                     u32 = 32;
pub const KDF_SALT_LEN:                     usize = 16;
pub const KDF_MEM_COST_KB:                  u32 = 104_858; // 100 MiB in KB
pub const KDF_TIME_COST_PASSES:             u32 = 5;

pub const WS_APP_MSG_LIMIT:                 u16 = 100; // Arbitrary.

// User interface.
pub const MAX_CREATE_PASS_ATTEMPTS:         usize = 3; // Arbitrary.
pub const SYNTAX_CMD_SIMILARITY_THRESHOLD:  f64 = 0.7;

// Development.
pub const DEV_TREE_CREATE: &[&str] = &[
    "tls/dev",
    "tls/prod",
    "www/logs",
    "www/public",
    "www/public/assets",
    "www/public/assets/font",
    "www/public/assets/img",
    "www/public/bundles",
    "www/public/bundles/js",
    "www/src/js",
    "www/src/js/components",
    "www/src/js/pages",
    "www/src/js/pages/main",
    "www/src/js/pages/admin",
    "www/src/js/utils",
    "www/src/styles",
    "www/src/styles/components",
];
pub const INIT_TREE_HALT: &[&str] = &[
    "www/public",
    "www/src",
];
