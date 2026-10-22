//! `pipe`, length-prefixed BDAT frames over a tokio byte stream: whole frames go through in order,
//! a declared length over the ceiling is refused before a byte of body is read or a buffer sized,
//! and a stream that ends part-way through a frame is a named fault rather than a short value.
#![cfg(feature = "pipe")]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    pipe::{
        FrameReader,
        FrameWriter,
        PipeFault,
        LEN_PREFIX,
    },
};

use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    time::{
        timeout,
        Duration,
    },
};


const MAX: usize = 1024 * 1024;

fn pair() -> (UnixStream, UnixStream) {
    match UnixStream::pair() {
        Ok(p)   => p,
        Err(e)  => panic!("A socketpair must open: {}", e),
    }
}

fn nth(i: usize) -> Dat {
    let mut m = DaticleMap::new();
    m.insert(dat!("i"), dat!(i as u64));
    m.insert(dat!("s"), dat!(fmt!("frame {}", i)));
    m.insert(dat!("b"), Dat::BU64(vec![(i % 251) as u8; i * 37 % 4099]));
    Dat::Map(m)
}

fn fault_of(r: Outcome<Option<Dat>>) -> Option<PipeFault> {
    match r {
        Ok(v)   => panic!("A fault was expected and the read gave {:?}.", v),
        Err(e)  => PipeFault::of(&e),
    }
}

#[tokio::test]
async fn test_thousand_frames_round_trip_over_a_socketpair_00() -> Outcome<()> {
    let (a, b) = pair();
    let mut w = FrameWriter::new(a, MAX);
    let mut r = FrameReader::new(b, MAX);
    let send = tokio::spawn(async move {
        for i in 0..1000 {
            res!(w.write(&nth(i)).await);
        }
        Ok::<(), Error<ErrTag>>(())
    });
    for i in 0..1000 {
        match res!(r.read().await) {
            Some(d) => assert_eq!(d, nth(i), "frame {} must arrive as sent", i),
            None    => panic!("The stream ended after {} of 1000 frames.", i),
        }
    }
    match send.await {
        Ok(o)   => res!(o),
        Err(e)  => panic!("The sender must finish: {}", e),
    }
    // The writer is gone, so the next read is a clean end between frames.
    assert_eq!(res!(r.read().await), None);
    Ok(())
}

#[tokio::test]
async fn test_over_max_header_refused_without_allocating_00() -> Outcome<()> {
    let (mut a, b) = pair();
    let mut r = FrameReader::new(b, MAX);
    // A prefix promising 4 GiB, and no body behind it on a stream left open: a reader that sized a
    // buffer or waited for the body would hang or fail the allocation, not answer.
    if let Err(e) = a.write_all(&u32::MAX.to_be_bytes()).await {
        panic!("{}", e);
    }
    let got = match timeout(Duration::from_secs(5), r.read()).await {
        Ok(g)   => g,
        Err(_)  => panic!("An over-max prefix must be refused at once, not waited on."),
    };
    assert_eq!(fault_of(got), Some(PipeFault::TooBig));
    // One byte over the ceiling is refused the same way.
    let (mut a, b) = pair();
    let mut r = FrameReader::new(b, MAX);
    if let Err(e) = a.write_all(&((MAX + 1) as u32).to_be_bytes()).await {
        panic!("{}", e);
    }
    assert_eq!(fault_of(r.read().await), Some(PipeFault::TooBig));
    drop(a);
    Ok(())
}

#[tokio::test]
async fn test_eof_mid_frame_is_a_typed_error_00() -> Outcome<()> {
    // Part-way through the body.
    let (mut a, b) = pair();
    let mut r = FrameReader::new(b, MAX);
    let body = res!(nth(7).as_bytes());
    if let Err(e) = a.write_all(&(body.len() as u32).to_be_bytes()).await { panic!("{}", e); }
    if let Err(e) = a.write_all(&body[..body.len() / 2]).await { panic!("{}", e); }
    drop(a);
    assert_eq!(fault_of(r.read().await), Some(PipeFault::Truncated));
    // Part-way through the prefix.
    let (mut a, b) = pair();
    let mut r = FrameReader::new(b, MAX);
    if let Err(e) = a.write_all(&[0u8; LEN_PREFIX - 2]).await { panic!("{}", e); }
    drop(a);
    assert_eq!(fault_of(r.read().await), Some(PipeFault::ShortPrefix));
    // After a whole frame, a torn second one still surfaces only as the fault.
    let (a, b) = pair();
    let mut w = FrameWriter::new(a, MAX);
    let mut r = FrameReader::new(b, MAX);
    res!(w.write(&nth(1)).await);
    let mut a = w.into_inner();
    if let Err(e) = a.write_all(&(body.len() as u32).to_be_bytes()).await { panic!("{}", e); }
    if let Err(e) = a.write_all(&body[..3]).await { panic!("{}", e); }
    drop(a);
    assert_eq!(res!(r.read().await), Some(nth(1)));
    assert_eq!(fault_of(r.read().await), Some(PipeFault::Truncated));
    Ok(())
}

#[tokio::test]
async fn test_writer_refuses_over_max_before_writing_00() -> Outcome<()> {
    let (a, b) = pair();
    let mut w = FrameWriter::new(a, 64);
    let mut r = FrameReader::new(b, 64);
    let big = Dat::BU64(vec![0u8; 65]);
    match w.write(&big).await {
        Ok(())  => panic!("A frame over the ceiling must be refused."),
        Err(e)  => assert_eq!(PipeFault::of(&e), Some(PipeFault::TooBig)),
    }
    res!(w.write(&dat!("small")).await);
    drop(w);
    // Nothing of the refused frame reached the stream.
    assert_eq!(res!(r.read().await), Some(dat!("small")));
    assert_eq!(res!(r.read().await), None);
    Ok(())
}

#[tokio::test]
async fn test_body_that_is_not_one_value_is_refused_00() -> Outcome<()> {
    let (mut a, b) = pair();
    let mut r = FrameReader::new(b, MAX);
    // Two values in one frame: the frame is one value, so the trailing one is an error.
    let mut body = res!(dat!(1u8).as_bytes());
    body.extend(res!(dat!(2u8).as_bytes()));
    if let Err(e) = a.write_all(&(body.len() as u32).to_be_bytes()).await { panic!("{}", e); }
    if let Err(e) = a.write_all(&body).await { panic!("{}", e); }
    // A body that is not BDAT at all.
    if let Err(e) = a.write_all(&3u32.to_be_bytes()).await { panic!("{}", e); }
    if let Err(e) = a.write_all(&[0xff, 0xff, 0xff]).await { panic!("{}", e); }
    drop(a);
    assert_eq!(fault_of(r.read().await), Some(PipeFault::Undecodable));
    assert_eq!(fault_of(r.read().await), Some(PipeFault::Undecodable));
    Ok(())
}
