//! # Shield (Signed Hash In Every Little Datagram)
//!
//! A security-focused peer-to-peer networking protocol built on UDP with comprehensive DoS 
//! resistance, post-quantum cryptography support, and flexible cryptographic scheme selection.
//!
//! ## Overview
//!
//! Shield implements a robust P2P protocol designed for hostile network environments, featuring:
//! - **Proof-of-work validation** with dynamic difficulty adjustment for DoS mitigation
//! - **3-stage handshake protocol** for secure session establishment
//! - **Post-quantum cryptography** options for future-proof security
//! - **Multi-layered guard system** with address and user-based protection
//! - **Flexible packet sizing** (700-1400 bytes) with automatic chunking for large messages
//! - **Generic protocol design** supporting custom ID lengths and cryptographic schemes
//!
//! ## Architecture
//!
//! The library is structured into two main modules:
//!
//! ### Server Protocol (`srv`)
//! Core protocol implementation with modular components:
//! - **Message system**: Packet handling, assembly, and handshake protocols
//! - **Guard system**: DoS protection with Monitor → Throttle → Blacklist state progression
//! - **Cryptographic schemes**: Pluggable encryption, signing, and hashing implementations
//! - **Proof-of-work engine**: Time-bounded PoW with linear difficulty scaling
//! - **Configuration management**: Runtime context and parameter tuning
//!
//! ### Application Layer (`app`)
//! High-level interfaces and tools:
//! - **Server wrapper**: Simplified server setup and management
//! - **REPL interface**: Interactive command processing
//! - **TUI support**: Text user interface components
//! - **Syntax parsing**: Command and configuration parsing
//!
//! ## Protocol Details
//!
//! ### Handshake Protocol
//! A 6-message exchange for secure session establishment. **Only the first
//! message of it is built**; see "Development status" below.
//! 1. **HReq1**: Initial request with signature public key
//! 2. **HResp1**: Server response with PoW challenge
//! 3. **HReq2**: Client authentication with PoW solution
//! 4. **HResp2**: Server KEM key exchange with session key
//! 5. **HReq3**: Client session confirmation
//! 6. **HResp3**: Server handshake completion
//!
//! ### Application payload exchange
//! A request and at most one answer, correlated by the message identifier in
//! the packet header and needing no session:
//! 1. **App request** (type 1,024): the caller's bytes, chunked across as many
//!    packets as they take, each carrying its own proof of work and signature.
//! 2. **App reply** (type 1,025): what the receiving peer's handler said,
//!    travelling under the identifier the request arrived with, back to the
//!    address it arrived from.
//!
//! Message types at or above 2,048 are the library user's own.
//!
//! ### Packet Structure
//! - **UDP buffer**: 1,400 bytes (avoiding IP fragmentation)
//! - **Default packet**: 700 bytes (substantial headroom)
//! - **Chunking threshold**: 1,500 bytes (split into 1,000-byte chunks)
//! - **Minimum chunk**: 42 bytes (accounts for encryption overhead)
//!
//! ### DoS Protection
//! Multi-layered defence with configurable thresholds:
//! - **Rate limiting**: 30 requests/second baseline with throttling
//! - **Proof-of-work**: 0-30 zero-bit difficulty scaling with request volume
//! - **Address blacklisting**: 30 minutes to 3 days with randomised duration
//! - **Message assembly limits**: 128 total repetitions, 32 per packet
//!
//! ## Cryptographic Features
//!
//! ### Supported Schemes
//! - **Encryption**: AES-GCM, ChaCha20-Poly1305, post-quantum options
//! - **Key Exchange**: Classical and post-quantum KEM implementations
//! - **Signatures**: RSA, ECDSA, EdDSA, post-quantum signature schemes
//! - **Hashing**: SHA-256, BLAKE3, argon2 for proof-of-work
//!
//! ### Security Properties
//! What the wire gives a caller **today**:
//! - **Per-packet proof of work**: every packet carries one, bound to both
//!   addresses, a challenge code and a timestamp within a horizon.
//! - **Per-packet signature**: every packet is signed, and an application
//!   payload carries the key it was signed with, because there is no session
//!   through which one could have been exchanged.
//! - **Rate limiting and blacklisting** per source address, on every packet.
//!
//! What the schemes are chosen for and the handshake would deliver, and which
//! **is not built yet**: forward secrecy through a KEM exchange, session
//! encryption, and post-quantum algorithm agility across all of it. A payload
//! that needs any of those must carry its own signature or its own encryption,
//! exactly as it would over an unencrypted stream.
//!
//! ## Usage Examples
//!
//! ### Carrying an application payload
//!
//! One peer listens and answers, the other dials and hears the answer on the
//! socket it dialled from. The bytes are the caller's; the protocol chunks,
//! proofs, signs and reassembles them without reading them.
//!
//! ```ignore
//! use oxedyne_fe2o3_shield::srv::{
//!     client::Client,
//!     constant,
//!     msg::{app::Answer, syntax as srv_syntax},
//!     server::Server,
//! };
//! use oxedyne_fe2o3_core::prelude::*;
//!
//! // The listening peer. `bind` hands back the socket before the loop starts,
//! // so the address it landed on can be read and told to somebody.
//! async fn echo(payload: Vec<u8>, _from: std::net::SocketAddr) -> Outcome<Answer> {
//!     Ok(Answer::Reply(payload))
//! }
//! let (mut server, _cmd) = Server::new(context, res!(srv_syntax::base_msg()));
//! let sock = res!(server.bind().await);
//! let addr = res!(sock.local_addr(), IO, Network);
//! tokio::spawn(async move { let _ = server.run(sock, echo).await; });
//!
//! // The dialling peer. A bind address of port zero is what a peer behind a
//! // household router wants: it dials out, and the answer comes back on the
//! // socket the question left on.
//! let client = res!(Client::bind(bind_addr, protocol, res!(srv_syntax::base_msg())).await);
//! let heard = res!(client.ask(addr, b"hello".to_vec(), constant::APP_REPLY_WAIT).await);
//! ```
//!
//! ### Custom cryptographic configuration
//!
//! The wire schemes are chosen when the protocol is built. Any field left
//! [`Alt::Unspecified`](oxedyne_fe2o3_core::alt::Alt) falls back to the
//! crate's default for it.
//!
//! ```ignore
//! use oxedyne_fe2o3_shield::srv::schemes::WireSchemesInput;
//! use oxedyne_fe2o3_crypto::{enc::EncryptionScheme, sign::SignatureScheme};
//! use oxedyne_fe2o3_core::{prelude::*, alt::Alt};
//!
//! let schemes = WireSchemesInput {
//!     enc:    Alt::Specific(None::<EncryptionScheme>),
//!     sign:   Alt::Specific(Some(SignatureScheme::new_ed25519())),
//!     ..Default::default()
//! };
//! ```
//!
//! ## Configuration Options
//!
//! Key parameters for tuning protocol behaviour:
//! - **Network**: UDP buffer size, packet sizes, chunking thresholds
//! - **Security**: PoW difficulty range, rate limiting thresholds
//! - **Session**: Handshake timeouts, session expiry intervals
//! - **Guard system**: Throttling limits, blacklist durations
//!
//! ## Performance Characteristics
//!
//! - **Throughput**: Optimised for 700-byte packets with minimal fragmentation
//! - **Latency**: 3-RTT handshake with configurable PoW difficulty
//! - **Memory**: ShardMap architecture for concurrent access scaling
//! - **CPU**: Efficient PoW validation with time-bounded challenges
//!
//! ## Development Status
//!
//! Built and exercised by tests:
//! - Multi-packet message assembly, and per-packet proof-of-work and signature
//!   validation.
//! - The address guard: rate limiting, throttling and blacklisting, on every
//!   packet whatever its type.
//! - An application payload path: request, reply and correlation, over a
//!   server that answers and a client that hears.
//! - Flexible cryptographic scheme selection.
//!
//! Not built:
//! - **The handshake beyond its first message.** `HReq1` is sent, received and
//!   recorded; `HResp1` has no encoder, and `HReq2`, `HResp2`, `HReq3` and
//!   `HResp3` exist as discriminants and syntax declarations and as no types at
//!   all. Nothing therefore establishes a session, and the session encryption
//!   the wire schemes carry is never applied.
//! - **A difficulty a peer can be told.** Because nothing answers `HReq1`, a
//!   dialling peer cannot learn the difficulty it is being asked for. A
//!   deployment must therefore fix the difficulty -- set
//!   `server_pow_zbits_min` equal to `server_pow_zbits_max` -- because a
//!   difficulty that rises with the request rate would silently stop a peer
//!   that has no way of hearing about the rise.
//!
//! APIs may change before the 1.0 release.
//!
//! ## Integration
//!
//! Shield integrates with the broader fe2o3 ecosystem:
//! - **fe2o3_crypto**: Cryptographic implementations and scheme selection
//! - **fe2o3_hash**: Hashing and proof-of-work functionality
//! - **fe2o3_net**: Network abstractions and protocol support
//! - **fe2o3_core**: Foundational error handling and data structures
//! - **fe2o3_jdat**: Serialisation and configuration management
//!
//! For detailed implementation examples and advanced configuration, see the
//! `examples/` directory and protocol specification documentation.
#![forbid(unsafe_code)]
pub mod app;
pub mod srv;
