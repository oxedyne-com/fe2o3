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
    collections::HashMap,
    //marker::{
    //    Send,
    //    Sync,
    //},
    sync::{
        Arc,
        Mutex,
    },
    thread,
};


pub trait LoggerConsole<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn new() -> Self;
    fn go(&mut self) -> SimplexThread<Msg<ETAG>>;
    fn listen(&mut self);
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
    fn new() -> Self {
        Self {
            chan: simplex(),
        }
    }

    fn go(&mut self) -> SimplexThread<Msg<ETAG>> {
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let chan_clone = self.chan.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            let mut logger = Self { chan: chan_clone };
            logger.listen();
        });
        SimplexThread::new(
            self.chan.clone(),
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        )
    }

    fn listen(&mut self) {
        while let Ok(msg) = self.chan.recv() {
            match msg {
                Msg::Finish(_src) => {
                    break;
                }
                Msg::Console((_stream, msg)) => {
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

#[derive(Clone, Debug)]
pub struct MultiStreamLoggerConsole<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub chan:   Simplex<Msg<ETAG>>,
    pub pools:  HashMap<String, Vec<String>>,
}

impl<ETAG: GenTag> LoggerConsole<ETAG> for MultiStreamLoggerConsole<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn new() -> Self {
        Self {
            chan: simplex(),
            pools: HashMap::new(),
        }
    }

    fn go(&mut self) -> SimplexThread<Msg<ETAG>> {
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let chan_clone = self.chan.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            let mut logger = Self {
                chan: chan_clone,
                pools: HashMap::new(),
            };
            logger.listen();
        });
        SimplexThread::new(
            self.chan.clone(),
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        )
    }

    fn listen(&mut self) {
        while let Ok(msg) = self.chan.recv() {
            match msg {
                Msg::Finish(_src) => {
                    break;
                }
                Msg::Console((stream, msg)) => {
                    self.pools.entry(stream).or_insert_with(Vec::new).push(msg);
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
