use oxedyne_fe2o3_o3db_sync::{
    comm::response::Wait,
};

use std::{
    time::Duration,
};

// Database.
pub const DB_DIR:                               &'static str = "o3db";
pub const GET_DATA_WAIT:                        Wait = Wait::new_default();

// Routes.
pub const DEFAULT_INDEX_FILE:                   &'static str = "index.html";

pub const HTTP_DEFAULT_HEADER_CHUNK_SIZE:       usize = 1_500;
pub const HTTP_DEFAULT_BODY_CHUNK_SIZE:         usize = 5_000;

pub const STACK_SIZE:                           usize = 2 * 1024 * 1024;

pub const SERVER_INT_CHANNEL_CHECK_INTERVAL:    Duration = Duration::from_nanos(1_000);
pub const SERVER_EXT_SOCKET_CHECK_INTERVAL:     Duration = Duration::from_secs(1);

// Retain enough passhashes to be satisfied that the user is not reusing recent passphrases.
pub const SERVER_RETAIN_PREV_PASSHASHES:        usize = 10;
// Retain archives of recent sessions.
pub const SERVER_RETAIN_PREV_SESSION_ARCHIVES:  usize = 10;
// WHen adding a new user, a new user id (UID) is randomly generated, which could collide with an
// existing UID, particularly for smaller UID lengths.  This value limits the number of
// regenerations.
pub const SERVER_MAX_UID_COLLISION_ATTEMPTS:    usize = 10;
// Generic message for unsuccessful login.
pub const SERVER_BAD_LOGIN_MSG:                 &'static str = "Login unsuccessful.";

// WebSocket.
pub const WEBSOCKET_CHUNK_SIZE:                 usize = 10;//4_096;
pub const WEBSOCKET_CHUNKING_THRESHOLD:         usize = 20;//16_384;

// rcgen TLS encryption schemes.
pub const PKCS_RSA_SHA256:                  &[u64] = &[1, 2, 840, 113549, 1, 1, 11];
pub const PKCS_RSA_SHA384:                  &[u64] = &[1, 2, 840, 113549, 1, 1, 12];
pub const PKCS_RSA_SHA512:                  &[u64] = &[1, 2, 840, 113549, 1, 1, 13];
pub const PKCS_RSA_PSS_SHA256:              &[u64] = &[1, 2, 840, 113549, 1, 1, 10];
pub const PKCS_ECDSA_P256_SHA256:           &[u64] = &[1, 2, 840, 10045, 4, 3, 2];
pub const PKCS_ECDSA_P384_SHA384:           &[u64] = &[1, 2, 840, 10045, 4, 3, 3];
pub const PKCS_ED25519:                     &[u64] = &[1, 3, 101, 112];

pub const UGRD_SHARDMAP_INIT_SHARDS:        usize = 10;
pub const AGRD_SHARDMAP_INIT_SHARDS:        usize = 10;
pub const GUARD_SHARDMAP_SALT_LEN:          usize = 8;
pub const SALT8: [u8; 8] = [
    0x13, 0x8b, 0x4f, 0xe3, 0xd3, 0x75, 0x67, 0x86,
];

// TLS certificate locations.
pub const TLS_DIR_DEV:                      &'static str = "dev";
pub const TLS_DIR_PROD:                     &'static str = "prod";

// Credits:
// Image: https://ascii-generator.site/
// Text: https://www.asciiart.eu/text-to-ascii-art Nancyj Improved with touch ups.
pub const SPLASH: &'static str =
r#"
   
    =***********************= 
   :*************************:
   ***************************
   ***=:::::::::::::::::::=***     88888888b                  .o88888o.         
   ***:                  :+***     88                        d8'     `8b        
   *******+=.       .=+*******     88aaaa   .d8888b.         88       88        
   **********       **********     88       88ooood8 d8888b. 88       88 d8888b.
   *********=       =*********     88       88.  ...     `88 Y8.     .8P     `88
   ******+:.  .---.  .:+******     dP       `88888P' .aaadP'  `888888P'   aaad8'
   ******+   -*****-   =******                       88'                     `88
   *******---*******---*******                       Y88888P             d88888P
   ***************************                                
    '^*********************^'                                   
                                                                Steel Web Server
   
"#;
