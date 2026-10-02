use oxedyne_fe2o3_core::{
    prelude::*,
    channels::{
        simplex,
        Recv,
        Simplex,
    },
    test::test_it,
};

use std::{
    sync::{
        atomic::{
            AtomicBool,
            AtomicUsize,
            Ordering,
        },
        Arc,
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
