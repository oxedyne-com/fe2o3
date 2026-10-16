//! Provides added convenience and ergonomics when using an existing channel implementation,
//! currently [flume](https://crates.io/crates/flume), whose channels can be used both blocking
//! and async, under any executor and on wasm32.  `Simplex` packages a transmission and receiver
//! pair for communicating in one direction, while `FullDuplex` packages two of these for
//! simultaneous bi-directional communication.
//!
//! For async actors there are also `watch`, a single-writer, many-reader cell holding the latest
//! value, and `oneshot`, a single reply.  Neither needs a runtime, and neither requires its value to
//! be `Send`, so both serve single-threaded executors (`spawn_local`, a tokio `LocalSet`) as well as
//! threads.  A bounded flume channel (`bounded`) is the mailbox: when full, `send_async` waits.

use crate::{
    prelude::*,
    //bot::CtrlMsg,
    time::wait_for_true,
};

use std::{
    fmt::Debug,
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{
        Arc,
        RwLock,
        RwLockReadGuard,
    },
    task::{
        Context,
        Poll,
        Waker,
    },
    time::{
        Duration,
        Instant,
    },
};

pub use flume::{
    bounded,
    unbounded,
    Sender,
    Receiver,
    RecvError,
    SendError,
    TryRecvError,
    TrySendError,
    RecvTimeoutError,
};

/// Waits until a message is waiting on any of the receivers, or until `timeout` has run out when
/// one is given, and returns the position of the receiver it came from.  `None` says the time ran
/// out first.  Nothing is polled: the caller sleeps until a sender wakes it, so a bot with several
/// channels to watch can block on all of them at once.
pub fn recv_any<M: 'static>(
    rxs:        &[&Receiver<M>],
    timeout:    Option<Duration>,
)
    -> Outcome<Option<(usize, M)>>
{
    let mut sel = flume::Selector::new();
    for (i, rx) in rxs.iter().enumerate() {
        sel = sel.recv(*rx, move |r| (i, r));
    }
    let (i, r) = match timeout {
        Some(d) => match sel.wait_timeout(d) {
            Ok(got) => got,
            Err(_)  => return Ok(None),
        },
        None => sel.wait(),
    };
    match r {
        Ok(m)   => Ok(Some((i, m))),
        Err(e)  => Err(err!(e,
            "While waiting on {} channels, the one at position {} was found disconnected.",
            rxs.len(), i;
            Channel, Read)),
    }
}

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

    /// Sends the message unless the channel has been closed, and says whether it was sent.  A
    /// `Simplex` and every clone of it hold the receiving end as well as the sending end, so
    /// `send` cannot fail because the reader has gone: it queues the message for nobody.  A
    /// reader that gives up closes the channel instead, and the open flag is held while a message
    /// is sent, so once `close` has returned no message can arrive that the closer's last
    /// `try_recv` has not seen.
    pub fn send_if_open(&self, msg: M) -> Outcome<bool> {
        let open = lock_read!(self.open,
            "While trying to send only if the channel is open.",
        );
        if !*open {
            return Ok(false);
        }
        res!(self.tx().send(msg));
        Ok(true)
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

// Watch.

/// Creates a latest-value cell holding `init`.  The single `WatchSender` replaces the value; any
/// number of `WatchReceiver`s borrow it and await the next change.  Dropping the sender closes it.
pub fn watch<T>(init: T) -> (WatchSender<T>, WatchReceiver<T>) {
    let shared = Arc::new(RwLock::new(WatchState {
        value:      init,
        version:    0,
        closed:     false,
        wakers:     Vec::new(),
    }));
    (
        WatchSender { shared: shared.clone() },
        WatchReceiver { shared, seen: 0 },
    )
}

struct WatchState<T> {
    value:      T,
    version:    u64,
    closed:     bool,
    wakers:     Vec<Waker>,
}

/// The single writer of a `watch` cell.  Not `Clone`.
pub struct WatchSender<T> {
    shared: Arc<RwLock<WatchState<T>>>,
}

impl<T> WatchSender<T> {

    /// Replaces the value and wakes every reader waiting in `changed`.  Succeeds whether or not any
    /// reader remains.
    pub fn send(&self, value: T) -> Outcome<()> {
        let wakers = {
            let mut state = lock_write!(self.shared, "While sending to a watch cell.");
            state.value = value;
            state.version = state.version.wrapping_add(1);
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
        Ok(())
    }

    /// A new reader that sees the current value as already seen.
    pub fn subscribe(&self) -> WatchReceiver<T> {
        let seen = match self.shared.read() {
            Ok(state)   => state.version,
            Err(p)      => p.into_inner().version,
        };
        WatchReceiver { shared: self.shared.clone(), seen }
    }

    pub fn borrow(&self) -> Outcome<WatchRef<'_, T>> {
        Ok(WatchRef(lock_read!(self.shared, "While borrowing a watch cell.")))
    }
}

impl<T> Drop for WatchSender<T> {
    fn drop(&mut self) {
        let wakers = {
            let mut state = match self.shared.write() {
                Ok(state)   => state,
                Err(p)      => p.into_inner(),
            };
            state.closed = true;
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }
}

/// A reader of a `watch` cell.  Each clone tracks what it has seen separately.
pub struct WatchReceiver<T> {
    shared: Arc<RwLock<WatchState<T>>>,
    seen:   u64,
}

impl<T> Clone for WatchReceiver<T> {
    fn clone(&self) -> Self {
        Self { shared: self.shared.clone(), seen: self.seen }
    }
}

impl<T> WatchReceiver<T> {

    /// The current value.  Hold the guard briefly: the sender waits while it is held.
    pub fn borrow(&self) -> Outcome<WatchRef<'_, T>> {
        Ok(WatchRef(lock_read!(self.shared, "While borrowing a watch cell.")))
    }

    /// Resolves once a value newer than the last one seen is sent, marking it seen.  A value sent
    /// before the sender dropped is still delivered; after that, it resolves to an error.
    pub fn changed(&mut self) -> WatchChanged<'_, T> {
        WatchChanged { rx: self }
    }

    /// Whether the sender has dropped.
    pub fn is_closed(&self) -> Outcome<bool> {
        Ok(lock_read!(self.shared, "While reading a watch cell.").closed)
    }
}

/// A read guard on the value of a `watch` cell.
pub struct WatchRef<'a, T>(RwLockReadGuard<'a, WatchState<T>>);

impl<'a, T> Deref for WatchRef<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { &self.0.value }
}

/// The future returned by `WatchReceiver::changed`.
pub struct WatchChanged<'a, T> {
    rx: &'a mut WatchReceiver<T>,
}

impl<'a, T> Future for WatchChanged<'a, T> {
    type Output = Outcome<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rx = &mut self.get_mut().rx;
        let mut state = match rx.shared.write() {
            Ok(state)   => state,
            Err(_)      => return Poll::Ready(Err(err!(
                "While awaiting a change in a watch cell: lock poisoned.";
            Channel, Lock, Poisoned))),
        };
        if state.version != rx.seen {
            rx.seen = state.version;
            return Poll::Ready(Ok(()));
        }
        if state.closed {
            return Poll::Ready(Err(err!(
                "Watch cell closed: its sender has dropped.";
            Channel, Read, Missing)));
        }
        if !state.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            state.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

// Oneshot.

/// Creates a single-use reply channel over a bounded(1) flume channel.
pub fn oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    let (tx, rx) = bounded(1);
    (OneshotSender(tx), OneshotReceiver(rx))
}

/// Sends one value, consuming itself.  Dropping it unsent is an error the receiver sees.
pub struct OneshotSender<T>(Sender<T>);

impl<T> OneshotSender<T> {
    pub fn send(self, value: T) -> Outcome<()> {
        match self.0.try_send(value) {
            Ok(()) => Ok(()),
            Err(_) => Err(err!(
                "Oneshot reply not sent: its receiver has dropped.";
            Channel, Write, Missing)),
        }
    }
}

/// Receives one value, consuming itself.
pub struct OneshotReceiver<T>(Receiver<T>);

impl<T> OneshotReceiver<T> {
    pub async fn recv(self) -> Outcome<T> {
        match self.0.recv_async().await {
            Ok(value)   => Ok(value),
            Err(e)      => Err(err!(e,
                "Oneshot reply never came: its sender dropped unsent.";
            Channel, Read, Missing)),
        }
    }
}
