//! Two peers on one machine, and the bytes that get from one to the other and
//! back.
//!
//! Everything here is the real wire: real chunking, a real proof of work on
//! every packet, a real Ed25519 signature over every packet, the real address
//! guard, and the real message assembler putting the pieces together. Nothing
//! is stubbed, because the parts that would be worth stubbing are exactly the
//! parts that have never been run against each other before.
//!
//! What is *not* here is the handshake. There is no session and nothing is
//! encrypted; the packet signature says the packet was signed by the key
//! travelling with it and no more. That is the same bar a length-prefixed
//! signed envelope over TCP clears, which is the point of this round.

use oxedyne_fe2o3_shield::srv::{
    cfg::ServerConfig,
    client::Client,
    constant,
    context::ServerContext,
    msg::{
        app::{
            Answer,
            AppMsgKind,
        },
        protocol::{
            DefaultProtocolTypes,
            Protocol,
            ProtocolMode,
        },
        syntax as srv_syntax,
    },
    schemes::WireSchemesInput,
    server::Server,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Alt,
    path::NormalPath,
    rand::RanDef,
};
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    sign::SignatureScheme,
};
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_net::id;
use oxedyne_fe2o3_o3db_sync::O3db;
use oxedyne_fe2o3_syntax::SyntaxRef;

use std::{
    net::SocketAddr,
    path::Path,
    time::Duration,
};


/// Length of the per-peer proof-of-work challenge code.
const CODE_LEN: usize = 8;

/// Bytes per chunk. Small, so that a payload of a few kilobytes is genuinely
/// carried by several packets and reassembled rather than fitting in one.
const CHUNK_BYTES: u64 = 400;

/// Proof-of-work difficulty, fixed: minimum and maximum are the same, so the
/// difficulty this peer demands does not move with the request rate.
///
/// A difficulty that moves is only usable once a peer can be *told* the new
/// one, and telling it is the handshake response that is still deferred. Until
/// then, a fixed difficulty is the honest setting, and this test uses one
/// rather than pretending otherwise.
const POW_ZBITS: u16 = 2;

/// How long a test waits for an answer before deciding none is coming.
const PATIENCE: Duration = Duration::from_secs(20);

/// How long a test waits when it expects nothing.
const QUIET: Duration = Duration::from_millis(500);

type Types = DefaultProtocolTypes<{ id::MID_LEN }, { id::SID_LEN }, { id::UID_LEN }>;

type Proto = Protocol<
    CODE_LEN,
    { id::MID_LEN },
    { id::SID_LEN },
    { id::UID_LEN },
    Types,
>;

/// The database type the server is parameterised by. No database is opened
/// here: the server needs the type to exist, not an instance of it.
type Db = O3db<
    { id::UID_LEN },
    id::Uid,
    EncryptionScheme,
    HashScheme,
    HashScheme,
    ChecksumScheme,
>;

type Srv = Server<
    CODE_LEN,
    { id::MID_LEN },
    { id::SID_LEN },
    { id::UID_LEN },
    Types,
    EncryptionScheme,
    HashScheme,
    Db,
>;

type Cli = Client<
    CODE_LEN,
    { id::MID_LEN },
    { id::SID_LEN },
    { id::UID_LEN },
    Types,
>;

/// A server configuration that binds loopback on whatever port is free.
fn loopback_config() -> ServerConfig {
    ServerConfig {
        server_address:         fmt!("127.0.0.1"),
        server_port_udp:        0,
        wire_chunk_bytes:       CHUNK_BYTES,
        wire_chunk_threshold:   CHUNK_BYTES,
        server_pow_zbits_min:   POW_ZBITS,
        server_pow_zbits_max:   POW_ZBITS,
        ..Default::default()
    }
}

/// A protocol instance with its own signing key and identifier.
///
/// Every peer signs with a key of its own, and carries the public half in each
/// packet: without a session there is nowhere else for the other side to have
/// got it from.
fn protocol(cfg: &ServerConfig, uid: u128) -> Outcome<Proto> {
    Protocol::new(
        cfg,
        WireSchemesInput {
            enc:    Alt::Specific(None::<EncryptionScheme>),
            csum:   Alt::Specific(None::<ChecksumScheme>),
            powh:   Alt::Specific(ServerConfig::default_packet_pow_hash_scheme()),
            sign:   Alt::Specific(Some(SignatureScheme::new_ed25519())),
            hsenc:  Alt::Specific(None::<EncryptionScheme>),
            chnk:   Some(ServerConfig::new_chunk_cfg(
                        CHUNK_BYTES as usize,
                        CHUNK_BYTES as usize,
                        false, // Do not wrap each chunk in a daticle.
                        false, // Do not pad the last chunk.
                    )),
        },
        [0u8; CODE_LEN],
        id::Mid::default(),
        id::Sid::default(),
        id::Uid::new(uid),
        ProtocolMode::Test,
    )
}

/// Build a server on loopback, hand back the address it landed on, and leave it
/// running with the given handler.
async fn serve<H, F>(handler: H) -> Outcome<SocketAddr>
where
    H: Fn(Vec<u8>, SocketAddr) -> F + Send + Sync + 'static,
    F: std::future::Future<Output = Outcome<Answer>> + Send,
{
    let cfg = loopback_config();
    let proto = res!(protocol(&cfg, 1));
    let root = Path::new(".").normalise().absolute();
    let context = ServerContext::new(cfg, root, None::<(Db, id::Uid)>, proto);
    let syntax = res!(srv_syntax::base_msg());
    let (mut server, _cmd_chan): (Srv, _) = Server::new(context, syntax);
    let sock = res!(server.bind().await);
    let addr = res!(sock.local_addr(), IO, Network);
    tokio::spawn(async move {
        if let Err(e) = server.run(sock, handler).await {
            error!(e);
        }
    });
    Ok(addr)
}

/// A client on loopback, on whatever port is free.
async fn dial(uid: u128) -> Outcome<(Cli, SyntaxRef)> {
    let cfg = loopback_config();
    let proto = res!(protocol(&cfg, uid));
    let syntax = res!(srv_syntax::base_msg());
    let addr = res!("127.0.0.1:0".parse::<SocketAddr>(), Test);
    let client = res!(Client::bind(addr, proto, syntax.clone()).await);
    Ok((client, syntax))
}

/// A handler that answers with the payload it was given, reversed.
///
/// Reversed rather than echoed, so that an answer which is somehow the
/// question coming back off a wire rather than out of the handler cannot pass.
async fn reverse(payload: Vec<u8>, _src_addr: SocketAddr) -> Outcome<Answer> {
    Ok(Answer::Reply(payload.into_iter().rev().collect()))
}

/// A payload of `n` bytes that is not the same at both ends and not the same
/// as any other length's.
fn payload(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

/// A payload larger than one chunk goes out in pieces and arrives whole, and
/// so does the answer to it.
#[tokio::test]
async fn test_payload_larger_than_one_chunk_00() -> Outcome<()> {
    log_set_level!("warn");
    let addr = res!(serve(reverse).await);
    let (client, _) = res!(dial(2).await);

    let sent = payload(2_000, 7);
    assert!(sent.len() > CHUNK_BYTES as usize * 4,
        "the point of this test is a payload of several chunks");
    let heard = res!(client.ask(addr, sent.clone(), PATIENCE).await);

    let mut want = sent.clone();
    want.reverse();
    assert_eq!(heard.len(), want.len(),
        "the answer came back a different length from the question");
    assert_eq!(heard, want, "the answer is not what the handler made of the question");

    // And a payload that fits in one chunk still works, so that what is being
    // proved above is the assembly and not merely that anything at all moves.
    let small = payload(16, 9);
    let heard = res!(client.ask(addr, small.clone(), PATIENCE).await);
    let mut want = small.clone();
    want.reverse();
    assert_eq!(heard, want);
    Ok(())
}

/// Two exchanges in the air at once, and each answer finds the question that
/// asked it.
///
/// The correlation is the message identifier in the packet header, and this is
/// what makes that load-bearing: an answer arriving under an identifier
/// nobody asked under is not an answer, however well-formed it is and however
/// much the waiting peer would like one.
#[tokio::test]
async fn test_reply_correlation_00() -> Outcome<()> {
    log_set_level!("warn");
    let addr = res!(serve(reverse).await);
    let (one, _) = res!(dial(3).await);
    let (two, _) = res!(dial(4).await);

    let first = payload(700, 11);   // Two chunks.
    let second = payload(700, 23);  // Two chunks, and different bytes.
    assert_ne!(first, second);

    let (heard_one, heard_two) = tokio::join!(
        one.ask(addr, first.clone(), PATIENCE),
        two.ask(addr, second.clone(), PATIENCE),
    );
    let heard_one = res!(heard_one);
    let heard_two = res!(heard_two);

    let mut want_one = first.clone();
    want_one.reverse();
    let mut want_two = second.clone();
    want_two.reverse();
    assert_eq!(heard_one, want_one, "the first peer was given the wrong answer");
    assert_eq!(heard_two, want_two, "the second peer was given the wrong answer");

    // An answer that arrives under an identifier this peer did not ask under is
    // dropped rather than taken. The question below really is answered -- the
    // server is alive and has just answered two others -- and the answer is
    // waited for under a fresh identifier that nothing was ever sent with, so
    // the only thing that can end this wait is the clock.
    let asked = res!(one.tell(addr, payload(32, 31)).await);
    let never = <id::Mid as RanDef>::randef();
    assert_ne!(never, asked, "the fresh identifier collided with a real one");
    match one.hear(&never, QUIET).await {
        Ok(bytes) => return Err(err!(
            "An answer of {} bytes was taken as the answer to a question that was \
            never asked.", bytes.len(); Test, Unexpected)),
        Err(_) => (),
    }
    Ok(())
}

/// Rubbish sent at the port is dropped, and the peer goes on working.
///
/// The loop is what is being tested, not the validator. A validator that
/// rejects a bad packet and a loop that dies doing it come to the same thing
/// from the outside, and the second is what an attacker would be aiming for.
#[tokio::test]
async fn test_garbage_is_dropped_00() -> Outcome<()> {
    log_set_level!("warn");
    let addr = res!(serve(reverse).await);

    // Every shape of rubbish that can reach a UDP port: nothing, too little to
    // be a header, a plausible length of noise, and a header whose chunk length
    // claims more bytes than the packet holds.
    let mut header_lie = payload(64, 3);
    header_lie[0] = 0x04; // Message type 1024, an application request.
    header_lie[1] = 0x00;
    for i in 33..37 {
        header_lie[i] = 0xff; // A chunk size of 65,535 in a 64-byte packet.
    }
    let rubbish: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0x00],
        payload(8, 1),
        payload(200, 2),
        header_lie,
        vec![0xff; constant::UDP_BUFFER_SIZE],
    ];
    let noise = res!(tokio::net::UdpSocket::bind("127.0.0.1:0").await, IO, Network);
    for r in &rubbish {
        res!(noise.send_to(r, addr).await, IO, Network);
    }

    // And the packet that is not rubbish at all except in one byte. This one is
    // built by the real builder -- real header, real proof of work, real
    // signature -- and then a single byte of the payload is flipped, which the
    // proof of work does not cover and the signature does. The control below it
    // is the same packet unflipped: without that, a silence here would only
    // show that something went wrong somewhere.
    let noise_addr = res!(noise.local_addr(), IO, Network);
    let cfg = loopback_config();
    let forger = res!(protocol(&cfg, 8));
    let syntax = res!(srv_syntax::base_msg());
    let honest = res!(forger.build_app(
        syntax.clone(),
        AppMsgKind::Request,
        <id::Mid as RanDef>::randef(),
        payload(64, 17),
        noise_addr.ip(),
        addr.ip(),
    ));
    assert_eq!(honest.len(), 1, "a 64-byte payload should fit in one packet");
    let mut tampered = honest[0].clone();
    let midpoint = tampered.len() / 2;
    tampered[midpoint] ^= 0x01;
    res!(noise.send_to(&tampered, addr).await, IO, Network);
    let mut heard = [0u8; constant::UDP_BUFFER_SIZE];
    match tokio::time::timeout(QUIET, noise.recv_from(&mut heard)).await {
        Err(_) => (), // Nothing came back, which is the point.
        Ok(_) => return Err(err!(
            "A packet whose payload was altered after it was signed was answered.";
            Test, Unexpected)),
    }

    // The control: the same packet, unaltered, is answered.
    let honest = res!(forger.build_app(
        syntax,
        AppMsgKind::Request,
        <id::Mid as RanDef>::randef(),
        payload(64, 17),
        noise_addr.ip(),
        addr.ip(),
    ));
    res!(noise.send_to(&honest[0], addr).await, IO, Network);
    match tokio::time::timeout(PATIENCE, noise.recv_from(&mut heard)).await {
        Ok(Ok(_)) => (),
        _ => return Err(err!(
            "The same packet unaltered was not answered, so the silence above \
            proves nothing."; Test, Unexpected)),
    }

    // The peer is still there, and still answers.
    let (client, _) = res!(dial(5).await);
    let sent = payload(500, 13);
    let heard = res!(client.ask(addr, sent.clone(), PATIENCE).await);
    let mut want = sent.clone();
    want.reverse();
    assert_eq!(heard, want, "the peer stopped answering after being sent rubbish");
    Ok(())
}

/// A server binds where its configuration says, and loopback is a thing the
/// configuration can say.
///
/// It could not before: the address was taken from the machine's network
/// interface whatever `server_address` held, which made two peers on one
/// machine -- a test, a development box -- impossible to arrange.
#[tokio::test]
async fn test_loopback_bind_00() -> Outcome<()> {
    log_set_level!("warn");
    let cfg = loopback_config();
    let proto = res!(protocol(&cfg, 6));
    let root = Path::new(".").normalise().absolute();
    let context = ServerContext::new(cfg, root, None::<(Db, id::Uid)>, proto);
    let syntax = res!(srv_syntax::base_msg());
    let (server, _cmd_chan): (Srv, _) = Server::new(context, syntax);
    let sock = res!(server.bind().await);
    let addr = res!(sock.local_addr(), IO, Network);
    assert!(addr.ip().is_loopback(),
        "a server told to bind 127.0.0.1 bound {} instead", addr.ip());
    assert!(addr.port() != 0, "a port of zero should have become a real one");

    // And a setting that is neither an address nor the word for the machine's
    // own is refused, rather than quietly becoming something else.
    let mut bad = loopback_config();
    bad.server_address = fmt!("not-an-address");
    assert!(bad.bind_ip().is_err(),
        "a server_address that is not an address was accepted");

    // The word an operator writes for "wherever this machine is on its network"
    // still resolves, which is what every deployed peer relies on.
    let mut here = loopback_config();
    here.server_address = fmt!("local");
    res!(here.bind_ip());
    Ok(())
}
