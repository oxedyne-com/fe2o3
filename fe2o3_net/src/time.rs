use oxedize_fe2o3_core::prelude::*;

use chrono::{
    Utc,
    TimeZone,
};

use std::time::Duration;

pub struct Time {}

impl Time {

    pub fn fmt_http(duration_since_epoch: &Duration) -> Outcome<String> {
        let datetime = match Utc.timestamp_opt(
            duration_since_epoch.as_secs() as i64,
            duration_since_epoch.subsec_nanos(),
        ).single() {
            Some(dt) => dt,
            None => {
                debug!("There was a problem with the supplied duration since the UNIX epoch:");
                debug!(" secs:  {}", duration_since_epoch.as_secs() as i64);
                debug!(" nanos: {}", duration_since_epoch.subsec_nanos());
                return Err(err!(
                    "The duration '{:?}' has not allowed construction of a valid date/time.",
                    duration_since_epoch;
                Invalid, Input, Mismatch));
            },
        };
    
        Ok(datetime.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
    }

}
