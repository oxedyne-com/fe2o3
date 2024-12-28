//! Provides added convenience and ergonomics when using an existing channel implementation,
//! currently [crossbeam-channel](https://crates.io/crates/crossbeam-channel).  `Simplex`
//! packages a transmission and receiver pair for communicating in one direction, while
//! `FullDuplex` packages two of these for simultaneous bi-directional communication.

use crate::{
    prelude::*,
    //bot::CtrlMsg,
    time::wait_for_true,
};

use std::{
    fmt::Debug,
    sync::{
        Arc,
        RwLock,
    },
    time::{
        Duration,
        Instant,
    },
};

pub use flume::{
    unbounded,
    Sender,
    Receiver,
    TryRecvError,
    RecvTimeoutError,
};

pub fn full_duplex<M>() -> FullDuplex<M> {
    FullDuplex (
        simplex(),
        simplex(),
    )
}

pub fn simplex<M>() -> Simplex<M> {
    let (tx, rx) = unbounded();
    Simplex {
        tx: tx,
        rx: rx,
        open: Arc::new(RwLock::new(true)),
    }
}

#[derive(Debug)]
/// A channel for communicating in a single direction.  It includes a thread-safe count of
/// pending messages.
///
/// ```ignore
///
///          tx ----->----- rx  A simplex channel is simple, just a
///                             transmitter end (tx) and a receiver
///                             end (rx).
///
/// ```
pub struct Simplex<M> {
    pub tx: Sender<M>,
    pub rx: Receiver<M>,
    open:   Arc<RwLock<bool>>, // Is the channel open for use?
}

impl<M> Clone for Simplex<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            rx: self.rx.clone(),
            open: self.open.clone(),
        }
    }
}

impl<M> Default for Simplex<M> {
    fn default() -> Self {
        simplex::<M>()
    }
}

impl<M> Simplex<M> {

    pub fn tx(&self) -> &Sender<M> { &self.tx }
    pub fn rx(&self) -> &Receiver<M> { &self.rx }

}

#[derive(Debug)]
pub enum Recv<M> {
    Result(Outcome<M>),
    Empty,
}

impl<M: 'static + Debug + Send + Sync> Simplex<M> {

    pub fn len(&self) -> usize {
        self.tx.len()
    }

    pub fn len_non_zero(&self) -> bool {
        self.len() > 0
    }

    /// Returns whether the value of the open flag.
    pub fn is_open(&self) -> Outcome<bool> {
        let open_read = lock_read!(self.open,
            "While trying to read whether channel is open.",
        );
        Ok(*open_read)
    }

    /// Sets the open flag to closed, and returns the existing status of the flag.  This simple
    /// mechanism starts the channel closing process by telling others to stop sending messages.
    pub fn close(&self) -> Outcome<bool> {
        let mut open_write = lock_write!(self.open,
            "While trying to close the channel.",
        );
        let is_open = *open_write;
        *open_write = false;
        Ok(is_open)
    }

    pub fn send(&self, msg: M) -> Outcome<()> {
        res!(self.tx().send(msg));
        Ok(())
    }

    /// Waits until a message is available.
    pub fn recv(&self) -> Outcome<M> {
        let msg = res!(self.rx().recv());
        Ok(msg)
    }

    /// Captures a message but does not wait until one is present.
    pub fn try_recv(&self) -> Recv<M> {
        match self.rx().try_recv() {
            Err(TryRecvError::Empty) => Recv::Empty,
            Err(e) => Recv::Result(Err(err!(e,
                "While trying to read channel without waiting.";
            Channel, Read))),
            Ok(msg) => Recv::Result(Ok(msg)),
        }
    }

    pub fn recv_timeout(&self, sleep: Duration) -> Recv<M> {
        match self.rx().recv_timeout(sleep) {
            Err(RecvTimeoutError::Timeout) => Recv::Empty,
            Err(e) => Recv::Result(Err(err!(e,
                "While reading channel with a timeout of {:?}.", sleep;
            Channel, Read))),
            Ok(msg) => Recv::Result(Ok(msg)),
        }
    }

    //pub fn send_ready(&self) -> Outcome<()> {
    //    self.send(M::ready())
    //}
    //pub fn send_finish(&self) -> Outcome<()> {
    //    self.send(M::finish())
    //}

    pub fn drain_messages(&self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            match self.try_recv() {
                Recv::Empty => break,
                Recv::Result(Err(e)) => lines.push(fmt!("<err>: {:?}", e)),
                Recv::Result(Ok(m)) => lines.push(fmt!("{:?}", m)),
            }
        }
        lines
    }

    /// Returns as soon as no more messages are detected in the channel.  Returns an error if the
    /// given `Duration`s are inconsistent.
    pub fn wait_for_empty_channel(
        &self,
        check_interval: Duration,
        max_wait:       Duration,
    ) 
        -> Outcome<(Instant, bool)>
    {
        wait_for_true(
            check_interval,
            max_wait,
            || { self.len() == 0 },
        )
    }
        
}

#[derive(Debug)]
/// A channel for communicating in a two directions.
///
/// ```ignore
///
///   fwd    tx1  ---->----  rx1    A full duplex contains two simplex
///                                 channels.  Each of these is accessed
///   rev    rx2  ----<----  tx2    via "fwd" and "rev".
///
/// ```
pub struct FullDuplex<M> (
    Simplex<M>,
    Simplex<M>,
);

impl<M> Clone for FullDuplex<M> {
    fn clone(&self) -> Self {
        Self (
            self.0.clone(),
            self.1.clone(),
        )
    }
}

impl<M> FullDuplex<M> {
    pub fn fwd(&self) -> &Simplex<M> { &self.0 }
    pub fn rev(&self) -> &Simplex<M> { &self.1 }
}

impl<M: 'static + Debug + Send + Sync> FullDuplex<M> {
    pub fn rx(&self) -> &Receiver<M> { &self.fwd().rx() }
    pub fn tx(&self) -> &Sender<M> { &self.fwd().tx() }

    pub fn send(&self, msg: M) -> Outcome<()> {
        self.fwd().send(msg)
    }

    pub fn recv(&self) -> Outcome<M> {
        self.fwd().recv()
    }
}

impl<M> Default for FullDuplex<M> {
    fn default() -> Self {
        full_duplex::<M>()
    }
}
