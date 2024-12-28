//!
//! A simple logging library.
//! 
//! Log a message by sending it to a static `LogBot` (lbot) thread using a macro.  An approximate
//! maximum file size can be set for the lbot file, which results in the creation of sequential
//! gzipped archive files over time.  See `tests/db.rs` for an example of how to boot and use the
//! lbot, run the test using `cargo test --test lbot -- --nocapture`.
//!
use crate::{
    prelude::*,
    channels::{
        simplex,
        Simplex,
    },
    log::{
        bot::{
            //LogBot,
            Msg,
        },
    },
    thread::{
        thread_channel,
        SimplexThread,
    },
};

use std::{
    fmt,
    marker::{
        Send,
        Sync,
    },
    sync::{
        Arc,
        Mutex,
    },
    thread,
};


pub trait LoggerConsole<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn go(&mut self) -> SimplexThread<Msg<ETAG>>;
    fn listen(&self);
}

#[derive(Clone, Debug)]
pub struct StdoutLoggerConsole<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub chan:   Simplex<Msg<ETAG>>,
}

impl<ETAG: GenTag> LoggerConsole<ETAG> for StdoutLoggerConsole<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn go(&mut self) -> SimplexThread<Msg<ETAG>> {
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let chan_clone = self.chan.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            let logger = Self { chan: chan_clone };
            logger.listen();
        });
        SimplexThread::new(
            self.chan.clone(),
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        )
    }

    fn listen(&self) {
        while let Ok(msg) = self.chan.recv() {
            match msg {
                Msg::Finish(_src) => {
                    //let msg = fmt!("Finish message received, logger console thread finishing now.");
                    //if let (Some(msg), _) = LogBot::format_msg(
                    //    LogLevel::Warn,
                    //    &src,
                    //    Ok(msg),
                    //    true,
                    //    false,
                    //) {
                    //    println!("{}", msg);
                    //}
                    break;
                }
                Msg::Console(msg) => {
                    println!("{}", msg)
                }
                _ => {
                    println!("{}", err!(
                        "Unexpected message type: {:?}", msg;
                    Bug, Unexpected, Input));
                }
            }
        }
    }
}

impl<ETAG: GenTag + fmt::Debug + Send + Sync> StdoutLoggerConsole<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub fn new() -> Self {
        Self {
            chan: simplex(),
        }
    }
}

