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
        ThreadController,
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
        RwLock,
    },
    thread,
};


pub fn switch_to_logger_console<
    L: LoggerConsole<ErrTag>,
>()
    -> Outcome<()>
{
    log_out_finish_wait!();
    let mut log_cfg = log_get_config!();
    let mut logger_console = L::new();
    let logger_console_thread = logger_console.go();

    // Update both channels:
    {
        let mut unlocked_chan_out = lock_write!(LOG.chan_out);
        *unlocked_chan_out = logger_console_thread.clone();
    }
    log_cfg.console = Some(logger_console_thread.chan.clone());

    log_set_config!(log_cfg);
    Ok(())
}

pub trait LoggerConsole<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn new() -> Self;
    fn go(&mut self) -> ThreadController<Msg<ETAG>>;
    fn listen(&mut self);
    fn get_streams(&self) -> HashMap<String, Simplex<String>> { HashMap::new() }
}

#[derive(Clone, Debug)]
pub struct StdoutLoggerConsole<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub chan:   Simplex<Msg<ETAG>>,
}

impl<ETAG: GenTag> LoggerConsole<ETAG> for StdoutLoggerConsole<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn new() -> Self {
        Self {
            chan: simplex(),
        }
    }

    fn go(&mut self) -> ThreadController<Msg<ETAG>> {
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let chan_clone = self.chan.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            let mut logger = Self { chan: chan_clone };
            logger.listen();
        });
        ThreadController::new(
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
                Msg::Console(_stream, msg) => {
                    println!("{}", msg)
                }
                _ => {
                    println!("{}", err!(
                        "Unexpected log message type: {:?}", msg;
                    Bug, Unexpected, Input));
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct MultiStreamLoggerConsole<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub chan:       Simplex<Msg<ETAG>>,
    pub streams:    Arc<RwLock<HashMap<String, Simplex<String>>>>,
}

impl<ETAG: GenTag> LoggerConsole<ETAG> for MultiStreamLoggerConsole<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn new() -> Self {
        Self {
            chan:       simplex(),
            streams:    Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn go(&mut self) -> ThreadController<Msg<ETAG>> {
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let chan_clone = self.chan.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            let mut logger = Self {
                chan:       chan_clone,
                streams:    Arc::new(RwLock::new(HashMap::new())),
            };
            logger.listen();
        });
        ThreadController::new(
            self.chan.clone(),
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        )
    }

    fn listen(&mut self) {
        while let Ok(msg) = self.chan.recv() {
            //match self.streams.read() {
            //    Ok(streams) =>  {
            //        let mut s = fmt!("");
            //        for (k, v) in streams.iter() {
            //            s.push_str(&fmt!(" k='{}' n={}",k,v.len()));
            //        }
            //        msg!("1000 {}", s);
            //    }
            //    Err(_) => println!("1000 Failed to acquire read lock on log streams."),
            //}
            match msg {
                Msg::Finish(_src) => {
                    break;
                }
                Msg::Console(stream, msg) => {
                    let chan = {
                        match self.streams.write() {
                            Ok(mut streams) => {
                                streams.entry(stream.clone())
                                    .or_insert_with(|| simplex())
                                    .clone()
                            }
                            Err(_) => {
                                println!("Failed to acquire write lock on log streams.");
                                continue;
                            }
                        }
                    };
                    if let Err(e) = chan.send(msg) {
                        println!("Failed to send to log message to stream '{}': {}",
                            stream, e);
                    }
                }
                Msg::AddStream(name, chan) => {
                    if let Err(_) = self.streams.write().map(|mut streams| {
                        streams.insert(name.clone(), chan);
                    }) {
                        println!("Failed to acquire write lock on log streams \
                            in order to add stream channel '{}'.", name);
                    }
                }
                Msg::GetStreams(responder) => {
                    //msg!("1500");
                    if let Err(e) = responder.send(self.streams.clone()) {
                        println!("{}", err!(e,
                            "While sending log streams map."; Channel, Write));
                    }
                    continue;
                }
                _ => {
                    println!("{}", err!(
                        "Unexpected log message type: {:?}", msg;
                        Bug, Unexpected, Input));
                }
            }
        }
    }
}
