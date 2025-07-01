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
//! 6-message exchange for secure session establishment:
//! 1. **HReq1**: Initial request with signature public key
//! 2. **HResp1**: Server response with PoW challenge
//! 3. **HReq2**: Client authentication with PoW solution
//! 4. **HResp2**: Server KEM key exchange with session key
//! 5. **HReq3**: Client session confirmation
//! 6. **HResp3**: Server handshake completion
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
//! - **Forward secrecy**: Session keys derived through KEM exchange
//! - **Replay protection**: Time-bounded PoW validation
//! - **Perfect forward secrecy**: Ephemeral key exchange
//! - **Post-quantum readiness**: Algorithm agility for quantum-resistant schemes
//!
//! ## Usage Examples
//!
//! ### Basic Server Setup
//! ```rust,no_run
//! use oxedyne_fe2o3_shield::srv::{Protocol, ProtocolMode};
//! use oxedyne_fe2o3_shield::srv::cfg::Config;
//! use oxedyne_fe2o3_core::prelude::*;
//!
//! // Create protocol with standard configuration.
//! let config = Config::default();
//! let protocol = res!(Protocol::new(ProtocolMode::Production, config));
//!
//! // Start UDP server.
//! res!(protocol.serve("0.0.0.0:8080").await);
//! # Ok::<(), Outcome<()>>(())
//! ```
//!
//! ### Custom Cryptographic Configuration
//! ```rust,no_run
//! use oxedyne_fe2o3_shield::srv::schemes::WireSchemes;
//! use oxedyne_fe2o3_crypto::{EncryptionScheme, SignatureScheme};
//! use oxedyne_fe2o3_core::prelude::*;
//!
//! // Configure custom cryptographic schemes.
//! let schemes = res!(WireSchemes::builder()
//!     .encryption(EncryptionScheme::ChaCha20Poly1305)
//!     .signature(SignatureScheme::Ed25519)
//!     .build());
//! # Ok::<(), Outcome<()>>(())
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
//! The library implements core Shield protocol functionality with:
//! - ✅ Complete handshake protocol implementation
//! - ✅ Multi-packet message assembly and validation
//! - ✅ Comprehensive DoS protection mechanisms
//! - ✅ Flexible cryptographic scheme support
//! - ✅ Proof-of-work engine with dynamic difficulty
//! - ⚠️  APIs may change before 1.0 release
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
