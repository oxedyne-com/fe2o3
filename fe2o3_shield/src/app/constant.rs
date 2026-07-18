use oxedyne_fe2o3_jdat::version::SemVer;
use oxedyne_fe2o3_o3db_sync::{
    comm::response::Wait,
};

/// Application version reported by the Shield server binary.
pub const VERSION:                          SemVer = SemVer::new(0, 1, 0);
/// Directory name, relative to the app root, holding the Ozone database.
pub const DB_DIR:                           &'static str = "o3db";
/// Default logging level when none is configured.
pub const DEFAULT_LOG_LEVEL:                &'static str = "info";
/// File extension used for log files.
pub const LOG_FILE_EXTENSION:               &'static str = "log";

/// File name of the application configuration document.
pub const CONFIG_NAME:                      &'static str = "config.jdat";
/// File name of the wallet document holding key material.
pub const WALLET_NAME:                      &'static str = "wallet.jdat";
// Retain enough passhashes to be satisfied that the user is not reusing recent passphrases.
/// Number of previous password hashes retained to detect passphrase reuse.
pub const NUM_PREV_PASSHASHES_TO_RETAIN:    usize = 10;
// Timeouts.
/// Default wait policy applied to database read operations.
pub const GET_DATA_WAIT:                    Wait = Wait::new_default();
// Key derivation functions.
/// Output length, in bytes, of the key-derivation function.
pub const KDF_HASH_LEN:                     u32 = 32;
/// Salt length, in bytes, supplied to the key-derivation function.
pub const KDF_SALT_LEN:                     usize = 16;
/// Memory cost, in kibibytes, of the key-derivation function (100 MiB).
pub const KDF_MEM_COST_KB:                  u32 = 104_858; // 100 MiB in KB
/// Number of passes performed by the key-derivation function.
pub const KDF_TIME_COST_PASSES:             u32 = 5;

/// Maximum queued application messages on a WebSocket connection.
pub const WS_APP_MSG_LIMIT:                 u16 = 100; // Arbitrary.

// User interface.
/// Number of attempts a user is given to create a valid passphrase.
pub const MAX_CREATE_PASS_ATTEMPTS:         usize = 3; // Arbitrary.
/// Minimum similarity score for suggesting a mistyped command.
pub const SYNTAX_CMD_SIMILARITY_THRESHOLD:  f64 = 0.7;

// Development.
/// Directory tree created when scaffolding a development deployment.
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
/// Directories whose contents are left untouched during re-initialisation.
pub const INIT_TREE_HALT: &[&str] = &[
    "www/public",
    "www/src",
];
