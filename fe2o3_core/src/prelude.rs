pub use crate::{
    self as oxedize_fe2o3_core,
    byte::B32,
    Outcome,
    GenTag,
    // Error handling and checking macros
    err,
    errmsg,
    ok,
    res,
    catch,
    catch_other,
    // Fallible operations and conversions
    try_into,
    try_add,
    try_sub,
    try_mul,
    try_div,
    try_rem,
    try_range,
    // Metaprogramming macros 
    new_enum,
    new_type,
    // String output macros
    dump,
    fmt,
    fmt_typ,
    msg,
    str,
    // Synchronisation macros
    lock_read,
    lock_write,
    // Test macros
    req,
    test_it,
};
pub use crate::error::{
    Error,
    ErrMsg,
    ErrTag,
};
// Logging.
pub use crate::{
    log,
    error,
    fault,
    warn,
    info,
    test,
    debug,
    trace,
    log_finish,
    log_finish_wait,
    log_in_finish_wait,
    log_out_finish_wait,
    get_log_level,
    get_log_config,
    set_log_level,
    set_log_config,
    set_log_out,
    get_log_file_path,
    log::{
        base::{
            LOG,
            LogLevel,
        },
        bot::{
            self as bot_log,
            LogBot,
        },
    },
};
// Traits
pub use std::str::FromStr; // Trait needed for log_level! macro.
pub use crate::conv::IntoInner;
