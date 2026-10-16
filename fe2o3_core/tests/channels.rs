use oxedyne_fe2o3_core::{
    prelude::*,
    channels::{
        bounded,
        oneshot,
        simplex,
        watch,
        Recv,
        Simplex,
    },
    test::test_it,
};

use std::{
    future::Future,
    pin::pin,
    rc::Rc,
    sync::{
        atomic::{
            AtomicBool,
            AtomicUsize,
            Ordering,
        },
        Arc,
    },
    task::{
        Context,
        Poll,
        Wake,
        Waker,
    },
    thread,
    time::Duration,
};

pub fn test_channels(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Send only if open 000", "all", "channels"], || {
        let chan: Simplex<u32> = simplex();
        let other = chan.clone();
        req!(true, res!(chan.send_if_open(1)), "(L: expected)");
        req!(true, res!(other.send_if_open(2)), "(L: expected)");
        // The flag is shared by every clone, and closing says it was open.
        req!(true, res!(other.close()), "(L: expected)");
        req!(false, res!(chan.is_open()), "(L: expected)");
        req!(false, res!(chan.send_if_open(3)), "(L: expected)");
        req!(false, res!(other.send_if_open(4)), "(L: expected)");
        // What went in before the close is still there to be read, in order, and nothing else.
        for want in [1, 2] {
            match chan.try_recv() {
                Recv::Result(Ok(got)) => req!(want, got),
                found => return Err(err!(
                    "Expected the message {} to be waiting, found {:?}.", want, found;
                    Test, Mismatch)),
            }
        }
        match chan.try_recv() {
            Recv::Empty => (),
            found => return Err(err!(
                "Expected nothing after the close, found {:?}.", found;
                Test, Mismatch)),
        }
        Ok(())
    }));

    // Why `send_if_open` is needed.  A `Simplex` holds its receiving end, so `send` succeeds
    // after every reader of the channel has gone, and the message waits for nobody.
    res!(test_it(filter, &["Send cannot see a reader gone 000", "all", "channels"], || {
        let chan: Simplex<u32> = simplex();
        let writer = chan.clone();
        drop(chan);
        req!(true, writer.send(1).is_ok(), "(L: expected)");
        req!(1, writer.len());
        Ok(())
    }));

    res!(test_it(filter, &["No send passes a close 000", "all", "channels"], || {
        for _ in 0..20 {
            let chan: Simplex<u32> = simplex();
            let go = Arc::new(AtomicBool::new(false));
            let sent = Arc::new(AtomicUsize::new(0));
            let mut hands = Vec::new();
            for _ in 0..6 {
                let (chan, go, sent) = (chan.clone(), go.clone(), sent.clone());
                hands.push(thread::spawn(move || {
                    while !go.load(Ordering::Acquire) {
                        thread::yield_now();
                    }
                    for i in 0..20_000u32 {
                        match chan.send_if_open(i) {
                            Ok(true) => { sent.fetch_add(1, Ordering::Relaxed); },
                            _ => break,
                        }
                    }
                }));
            }
            go.store(true, Ordering::Release);
            thread::sleep(Duration::from_micros(300));
            res!(chan.close());
            // Every message that reported itself sent was in the queue by the time the close
            // returned, so a reader that drains after closing has them all, and no more come.
            let at_close = chan.len();
            for hand in hands {
                let joined = hand.join().is_ok();
                req!(true, joined, "(L: expected)");
            }
            req!(at_close, chan.len());
            req!(sent.load(Ordering::Relaxed), chan.len());
        }
        Ok(())
    }));

    Ok(())
}

/// Records that it was woken, so a test can poll by hand and see exactly when a future is ready.
struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) { self.0.store(true, Ordering::SeqCst); }
}

fn flag() -> (Arc<Flag>, Waker) {
    let f = Arc::new(Flag(AtomicBool::new(false)));
    (f.clone(), Waker::from(f))
}

/// The smallest executor: park the thread until woken.
struct Unpark(thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) { self.0.unpark(); }
}

fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => thread::park(),
        }
    }
}

#[test]
fn mailbox_full_makes_sender_wait() -> Outcome<()> {
    let (tx, rx) = bounded::<usize>(64);
    for i in 0..64 {
        res!(tx.try_send(i));
    }
    let (woken, waker) = flag();
    let mut cx = Context::from_waker(&waker);
    let mut send = pin!(tx.send_async(64));
    req!(send.as_mut().poll(&mut cx).is_pending(), true);
    req!(tx.len(), 64);
    req!(woken.0.load(Ordering::SeqCst), false);
    // A second poll while still full stays pending: the mailbox never grows.
    req!(send.as_mut().poll(&mut cx).is_pending(), true);
    req!(tx.len(), 64);
    req!(res!(rx.try_recv()), 0);
    req!(woken.0.load(Ordering::SeqCst), true);
    match send.as_mut().poll(&mut cx) {
        Poll::Ready(r) => res!(r),
        Poll::Pending => return Err(err!("65th send still pending after a slot was freed."; Test)),
    }
    req!(tx.len(), 64);
    for i in 1..65 {
        req!(res!(rx.try_recv()), i);
    }
    Ok(())
}

#[test]
fn watch_late_reader_sees_latest() -> Outcome<()> {
    let (tx, rx) = watch(0u32);
    for i in 1..4 {
        res!(tx.send(i));
    }
    req!(*res!(rx.borrow()), 3);
    let late = tx.subscribe();
    req!(*res!(late.borrow()), 3);
    let cloned = late.clone();
    req!(*res!(cloned.borrow()), 3);
    Ok(())
}

#[test]
fn watch_changed_wakes_on_send() -> Outcome<()> {
    let (tx, mut rx) = watch(String::from("idle"));
    {
        let (woken, waker) = flag();
        let mut cx = Context::from_waker(&waker);
        let mut changed = pin!(rx.changed());
        req!(changed.as_mut().poll(&mut cx).is_pending(), true);
        res!(tx.send(String::from("stop")));
        req!(woken.0.load(Ordering::SeqCst), true);
        match changed.as_mut().poll(&mut cx) {
            Poll::Ready(r) => res!(r),
            Poll::Pending => return Err(err!("changed() still pending after send."; Test)),
        }
    }
    req!(res!(rx.borrow()).as_str(), "stop");
    // Seen: the next changed() waits again.
    let (_woken, waker) = flag();
    let mut cx = Context::from_waker(&waker);
    req!(pin!(rx.changed()).poll(&mut cx).is_pending(), true);
    // Across threads, with a parking executor.
    let handle = thread::spawn(move || -> Outcome<()> {
        res!(tx.send(String::from("again")));
        Ok(())
    });
    res!(block_on(rx.changed()));
    req!(res!(rx.borrow()).as_str(), "again");
    match handle.join() {
        Ok(r) => res!(r),
        Err(_) => return Err(err!("Sender thread panicked."; Test)),
    }
    Ok(())
}

#[test]
fn watch_closes_when_sender_drops() -> Outcome<()> {
    let (tx, mut rx) = watch(1u8);
    let (woken, waker) = flag();
    let mut cx = Context::from_waker(&waker);
    {
        let mut changed = pin!(rx.changed());
        req!(changed.as_mut().poll(&mut cx).is_pending(), true);
        res!(tx.send(2));
        drop(tx);
        req!(woken.0.load(Ordering::SeqCst), true);
        // The last value sent before the drop is still delivered.
        match changed.as_mut().poll(&mut cx) {
            Poll::Ready(r) => res!(r),
            Poll::Pending => return Err(err!("changed() pending after send."; Test)),
        }
    }
    req!(*res!(rx.borrow()), 2);
    req!(res!(rx.is_closed()), true);
    req!(block_on(rx.changed()).is_err(), true);
    Ok(())
}

/// The actor runs single-threaded (spawn_local on wasm, LocalSet natively), so a watch must carry a
/// value that is not `Send`.  This compiling and passing is why there is no separate non-`Send`
/// variant.
#[test]
fn watch_carries_non_send_value() -> Outcome<()> {
    let (tx, mut rx) = watch(Rc::new(0u8));
    res!(tx.send(Rc::new(7)));
    res!(block_on(rx.changed()));
    req!(**res!(rx.borrow()), 7);
    Ok(())
}

#[test]
fn oneshot_delivers_value() -> Outcome<()> {
    let (tx, rx) = oneshot::<u64>();
    res!(tx.send(42));
    let got = res!(block_on(rx.recv()));
    req!(got, 42);
    Ok(())
}

#[test]
fn oneshot_dropped_sender_is_error() -> Outcome<()> {
    let (tx, rx) = oneshot::<u64>();
    drop(tx);
    let got = block_on(rx.recv());
    req!(got.is_err(), true);
    Ok(())
}

#[test]
fn oneshot_waits_for_sender_thread() -> Outcome<()> {
    let (tx, rx) = oneshot::<&'static str>();
    let handle = thread::spawn(move || tx.send("done"));
    let got = res!(block_on(rx.recv()));
    req!(got, "done");
    match handle.join() {
        Ok(r) => res!(r),
        Err(_) => return Err(err!("Sender thread panicked."; Test)),
    }
    Ok(())
}
