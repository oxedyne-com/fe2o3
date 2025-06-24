use oxedyne_fe2o3_core::prelude::*;

use std::{
    thread,
    time::Duration,
};

pub fn delayed_error<T: GenTag>(e: Error<T>, delay: u64) -> Error<T> {
    thread::sleep(Duration::from_secs(delay));
    e
}

