# Hematite (fe2o3) Library Overview

**Version:** 0.5.0 (pre-1.0.0, ~50% complete)
**Licence:** BSD-2-Clause
**Repository:** https://github.com/Oxedize/fe2o3
**Author:** h00gs <hello@oxedize.com>

## Project Summary

Hematite is a collection of Rust crates that grew from an exploration into database design and key data structures. The project is built from first principles with a focus on readability, correctness and maintainability over cleverness and premature optimisation. It avoids `unsafe` code and `unwrap()` throughout, using custom error handling macros (`res!`, `ok!`, `err!`, `catch!`) via the `Outcome<T>` result type.

The library currently comprises 24 crates (including 2 procedural macro crates), organised by their level of cross-dependency into foundational, fundamental, functional and application-level tiers.

---

## Architecture and Dependency Graph

The crates form a layered dependency graph. `fe2o3_core` sits at the base and is used by every other crate. `fe2o3_jdat` (the serialisation format) is the next most widely depended upon, followed by `fe2o3_namex` (the naming registry).

```
                        Application Layer
        ┌──────────┬──────────┬──────────┐
        │ fe2o3_   │ fe2o3_   │ fe2o3_   │
        │ o3db     │ shield   │ steel    │
        └────┬─────┴────┬─────┴────┬─────┘
             │          │          │
                    Functional Layer
    ┌────────┬──────────┬──────────┬──────────┐
    │ fe2o3_ │ fe2o3_   │ fe2o3_   │ fe2o3_   │
    │ net    │ tui      │ crypto   │ hash     │
    └───┬────┴────┬─────┴────┬─────┴────┬─────┘
        │         │          │          │
                   Fundamental Layer
    ┌────────┬──────────┬──────────┬──────────┐
    │ fe2o3_ │ fe2o3_   │ fe2o3_   │ fe2o3_   │
    │ jdat   │ syntax   │ namex    │ bot      │
    ├────────┼──────────┼──────────┼──────────┤
    │ fe2o3_ │ fe2o3_   │ fe2o3_   │ fe2o3_   │
    │ data   │ file     │ text     │ units    │
    └───┬────┴────┬─────┴────┬─────┴────┬─────┘
        │         │          │          │
                   Foundational Layer
    ┌────────┬──────────┬──────────┬──────────┐
    │ fe2o3_ │ fe2o3_   │ fe2o3_   │ fe2o3_   │
    │ core   │ stds     │ geom     │ test     │
    └────────┴──────────┴──────────┴──────────┘

         Interoperability Protocol (IOP) Layer
    ┌────────────┬──────────────┬──────────────┐
    │ fe2o3_     │ fe2o3_       │ fe2o3_       │
    │ iop_crypto │ iop_hash     │ iop_db       │
    └────────────┴──────────────┴──────────────┘
```

---

## Crate-by-Crate Overview

### Foundational Crates

These have no internal cross-dependencies (or only depend on `fe2o3_stds`).

#### fe2o3_stds -- Standard Data Enumerations

**Purpose:** Provides standard data enumerations used across the library.

**Current capabilities:**
- `chars` -- character enumerations and classification.
- `regions` -- geographic and regional enumerations.

**What is next:**
- No known outstanding items.

**Tests:** None.

---

#### fe2o3_core -- Core Traits and Utilities

**Purpose:** The foundational crate upon which all others depend. Provides the custom error handling system, logging, macros and core utilities.

**Dependencies:** `fe2o3_stds`, `new` (proc-macro), `base64`, `flume`, `rand`, `flate2`, `humantime`, `once_cell`.

**Current capabilities:**
- `error` -- the `Outcome<V>` result type and `GenTag` error tag trait.
- `log` -- logging framework.
- `macros/` -- a comprehensive collection of macros:
  - `error` -- `res!`, `ok!`, `err!`, `catch!` for error handling.
  - `lock` -- `lock_read!`, `lock_write!`, `lock_mutex!`, `lock_mutex_thread!` for safe lock acquisition.
  - `collection`, `conversion`, `enum_iter`, `integer`, `newtype`, `range`, `string`, `test`.
- `alt` -- alternative/option utilities.
- `bool`, `bot`, `byte` -- primitive type extensions.
- `channels` -- channel-based communication helpers.
- `conv` -- type conversion utilities.
- `count` -- counting utilities.
- `file` -- basic file operations.
- `id` -- identification utilities.
- `int` -- integer utilities.
- `map` -- map/collection helpers.
- `mem` -- memory utilities.
- `ord` -- ordering utilities.
- `path` -- path manipulation.
- `rand` -- random number generation.
- `string` -- string utilities.
- `test` -- test helpers.
- `thread` -- threading utilities.
- `time` -- time utilities.

**What is next:**
- No critical outstanding items identified.

**Tests:** 3 test files (`main.rs`, `path.rs`, `string.rs`).

---

#### fe2o3_geom -- Geometry Library

**Purpose:** Basic geometry primitives.

**Dependencies:** `fe2o3_core`.

**Current capabilities:**
- `dim` -- dimensional types.
- `rect` -- rectangle types and operations.

**What is next:**
- No known outstanding items.

**Tests:** 1 test file (`macro.rs`).

---

#### fe2o3_test -- Testing Utilities

**Purpose:** Testing and performance measurement utilities used across the library.

**Dependencies:** `fe2o3_core`, `rand`.

**Current capabilities:**
- `data` -- test data generation.
- `error` -- test error utilities.

**What is next:**
- No known outstanding items.

**Tests:** None (utility crate).

---

### Fundamental Crates

These have 1-3 internal cross-dependencies.

#### fe2o3_num -- Numerical Type Utilities

**Purpose:** Extended numerical types and utilities beyond the standard library.

**Dependencies:** `fe2o3_core`, `bigdecimal` 0.2.0, `num-bigint` 0.3.

**Current capabilities:**
- `float` -- floating point utilities.
- `int` -- integer utilities and extensions.
- `string` -- number-to-string formatting.
- `macros` -- numerical macros.
- Re-exports `BigInt` and `BigDecimal`.

**What is next:**
- No known outstanding items.

**Tests:** 1 test file (`macro.rs`).

---

#### fe2o3_text -- String Manipulation and Formatting

**Purpose:** Rich text processing, encoding and pattern matching.

**Dependencies:** `fe2o3_core`, `fe2o3_geom`, `fe2o3_stds`, `base64`.

**Current capabilities:**
- `Text` struct -- core text type.
- `access` -- text access and extraction.
- `base2x` -- base-2x encoding/decoding.
- `core` -- core text operations.
- `highlight` -- syntax/text highlighting.
- `lines` -- line-by-line processing.
- `pattern` -- pattern matching.
- `split` -- text splitting.
- `string` -- string extensions.
- `phrase` -- phrase-level operations.

**What is next:**
- No known outstanding items.

**Tests:** 5 test files (`base2x.rs`, `highlight.rs`, `main.rs`, `pattern.rs`, `string.rs`).

---

#### fe2o3_units -- Scientific Units Library

**Purpose:** SI and other unit system representations with scaling.

**Dependencies:** `fe2o3_core`, `fe2o3_num`.

**Current capabilities:**
- `si` -- SI (International System) unit definitions.
- `system` -- unit system framework.
- `scale` -- unit scaling and conversion (partially implemented).

**What is next:**
- Scale lookup functions in `scale.rs` contain `unimplemented!()` calls (lines 249, 261) that need completing.

**Tests:** None.

---

#### fe2o3_bot -- Thread Worker Library

**Purpose:** Thread worker and task pool management using OS threads.

**Dependencies:** `fe2o3_core`, `fe2o3_jdat`.

**Current capabilities:**
- `Bot` -- the core worker type.
- `handles` -- thread handle management.
- `msg` -- inter-bot messaging.

**What is next:**
- No known outstanding items.

**Tests:** None.

---

#### fe2o3_data -- Specialised Data Structures

**Purpose:** Data structures beyond the standard library.

**Dependencies:** `fe2o3_core`, `fe2o3_jdat`.

**Current capabilities:**
- `ring` -- ring buffer.
- `stack` -- stack data structure.
- `time` -- time-related data structures.
- `tree` -- tree data structures.

**What is next:**
- No known outstanding items.

**Tests:** 3 test files (`base2x.rs`, `main.rs`, `path.rs`).

---

#### fe2o3_file -- File System Utilities

**Purpose:** File system operations and directory tree management.

**Dependencies:** `fe2o3_core`, `fe2o3_data`.

**Current capabilities:**
- `tree` -- directory tree traversal and operations.

**What is next:**
- No known outstanding items.

**Tests:** 2 test files (`main.rs`, `tree.rs`).

---

### Interoperability Protocol (IOP) Crates

These define abstract interfaces (traits) that separate specification from implementation, allowing different concrete implementations to be swapped in.

#### fe2o3_iop_crypto -- Cryptography Interoperability

**Purpose:** Defines abstract interfaces for cryptographic operations.

**Dependencies:** `fe2o3_core`, `fe2o3_namex`.

**Current capabilities:**
- `enc` -- encryption interface.
- `kem` -- key encapsulation mechanism interface.
- `keys` -- key management interface.
- `sign` -- digital signature interface.

**What is next:**
- No known outstanding items.

**Tests:** None (interface-only crate).

---

#### fe2o3_iop_hash -- Hashing Interoperability

**Purpose:** Defines abstract interfaces for hashing operations.

**Dependencies:** `fe2o3_core`, `fe2o3_namex`.

**Current capabilities:**
- `api` -- core hashing API traits.
- `csum` -- checksum interface.
- `kdf` -- key derivation function interface.

**What is next:**
- No known outstanding items.

**Tests:** None (interface-only crate).

---

#### fe2o3_iop_db -- Database Interoperability

**Purpose:** Defines abstract interfaces for database operations.

**Dependencies:** `fe2o3_core`, `fe2o3_crypto`, `fe2o3_data`, `fe2o3_hash`, `fe2o3_iop_crypto`, `fe2o3_iop_hash`, `fe2o3_jdat`, `fe2o3_namex`.

**Current capabilities:**
- `api` -- common database interface traits.

**What is next:**
- No known outstanding items.

**Tests:** None (interface-only crate).

---

### Functional Crates

These have 4 or more cross-dependencies and provide significant standalone functionality.

#### fe2o3_jdat -- JDAT Format (Jason's Data And Type)

**Purpose:** A superset of JSON that adds type annotations, binary serialisation and structured key support. This is one of the most central crates in the library, used by ~15 other crates.

**Dependencies:** `fe2o3_core`, `fe2o3_num`, `fe2o3_text`, `dat_map` (proc-macro), `bigdecimal`, `num-bigint`.

**Current capabilities:**
- `Dat` enum -- the core data type with rich variant set.
- `Kind` -- type descriptor system.
- `Daticle` trait -- core serialisation/deserialisation trait.
- `binary/` -- binary encoding and decoding:
  - `enc.rs` -- binary encoder.
  - `dec.rs` -- binary decoder.
  - `core.rs` -- shared binary logic.
  - `load.rs` -- binary loading.
  - `count.rs` -- byte counting.
- `string/` -- human-readable string encoding and decoding:
  - `enc.rs` -- string encoder.
  - `dec.rs` -- string decoder.
  - `core.rs` -- shared string logic.
- `map` -- map operations with arbitrary key types.
- `conv` -- type conversion utilities.
- `file` -- JDAT file I/O.
- `id` -- identification types.
- `int` -- integer handling.
- `note` -- annotations and comments.
- `usr` -- user-defined type support.
- `version` -- versioning support.
- `constant` -- format constants.
- `cfg` -- configuration support.
- `chunk` -- chunked data handling.
- Key traits: `BestFrom` (conversion), `FromDatMap`, `ToDatMap`.

**Key features over JSON:**
- Type annotations (e.g., `(u64) 42`).
- Multiple number formats (integers, floats, big numbers).
- Binary representation for compact storage.
- Comment support.
- Any type as map keys (not just strings).

**What is next:**
- B256 type consideration (noted in `conv.rs` line 458).

**Benchmarks:** 2 benchmark files.
**Tests:** 5 test files (`byte.rs`, `daticle.rs`, `main.rs`, `map.rs`, `string.rs`).

---

#### fe2o3_namex -- Universal Name Codex

**Purpose:** A distributed naming system for schemes, specifications and identifiers. Used by ~8 other crates.

**Dependencies:** `fe2o3_core`, `fe2o3_jdat`, `fe2o3_text`, `base64`, `strum`, `strum_macros`, `num-derive`, `num-traits`.

**Current capabilities:**
- `InNamex` trait -- interface for named/registered items.
- `db` -- database/registry functions.
- `id` -- identification and naming.

**What is next:**
- Date validation needs completing (`db.rs` lines 295, 301).

**Tests:** 3 test files (`file.rs`, `genids.rs`, `main.rs`).

---

#### fe2o3_hash -- Generic Hashing Utilities

**Purpose:** Wrappers for various hash schemes, conforming to the `fe2o3_iop_hash` interfaces.

**Dependencies:** `fe2o3_core`, `fe2o3_iop_hash`, `fe2o3_jdat`, `fe2o3_namex`, `crc32fast`, `seahash`, `tiny-keccak`, `rust-argon2`, `num_cpus`, `base64`.

**Current capabilities:**
- `csum` -- checksum implementations (CRC32).
- `hash` -- core hashing (SeaHash, Keccak/SHA-3).
- `kdf` -- key derivation functions (Argon2).
- `map` -- hash map utilities.
- `pow` -- proof-of-work computation.

**What is next:**
- Async thread optimisation for proof-of-work (`pow.rs` line 375).

**Tests:** 3 test files (`hash.rs`, `main.rs`, `map.rs`).

---

#### fe2o3_crypto -- Post-Quantum Cryptography

**Purpose:** Implements post-quantum cryptographic algorithms from the NIST PQC standardisation process, conforming to `fe2o3_iop_crypto` interfaces.

**Dependencies:** `fe2o3_core`, `fe2o3_data`, `fe2o3_iop_crypto`, `fe2o3_jdat`, `fe2o3_namex`, `aes-gcm`, `ed25519-dalek`, `pqcrypto-dilithium`, `secrecy`, `zeroize`, `wasm-bindgen`.

**Library type:** `cdylib` + `lib` (supports WebAssembly).

**Feature flags:** `mode0`, `mode1`, `mode2` (default), `mode3`.

**Current capabilities:**
- `pqc/dilithium` -- CRYSTALS-Dilithium digital signatures.
- `pqc/saber` -- SABER key encapsulation mechanism.
- `enc` -- AES-GCM symmetric encryption.
- `kem` -- key encapsulation mechanism framework.
- `keys` -- key management.
- `scheme` -- cryptographic scheme definitions.
- `sign` -- ED25519 elliptic curve signatures.
- `wasm/` -- WebAssembly bindings for browser use.

**Build:** Uses `bindgen` for C FFI (SABER reference implementation).

**What is next:**
- Dilithium comparison optimisation (`dilithium.rs` line 1586).
- Subtle timing-safe comparison needed (`dilithium.rs` line 1281).
- SABER variant implementations incomplete (`saber.rs` line 2322).

**Tests:** 1 test file.

---

#### fe2o3_syntax -- Command and Message Syntax

**Purpose:** A unified syntax definition and parsing system that bridges CLI/TUI commands and over-the-wire message protocols (OSI Presentation Layer).

**Dependencies:** `fe2o3_core`, `fe2o3_jdat`, `fe2o3_stds`, `fe2o3_text`, `fe2o3_units`, `levenshtein`.

**Current capabilities:**
- `Syntax`, `SyntaxRef` -- core syntax definition types.
- `cmd` -- command definitions.
- `arg` -- argument parsing.
- `opt` -- option handling.
- `msg` -- message serialisation/deserialisation.
- `key` -- keyword handling.
- `help` -- help text generation.
- `core` -- core parsing logic.
- `apps` -- application-level syntax.
- Builder pattern for defining command structures.
- Levenshtein distance for "did you mean?" suggestions.

**What is next:**
- Help system completion (`help.rs` lines 57, 269).
- Message serialisation optimisations (`msg.rs` line 147).

**Tests:** 3 test files (`core.rs`, `main.rs`, `msg.rs`).

---

#### fe2o3_tui -- Text User Interface Library

**Purpose:** A terminal-based user interface library with REPL support.

**Dependencies:** `fe2o3_core`, `fe2o3_file`, `fe2o3_geom`, `fe2o3_hash`, `fe2o3_iop_hash`, `fe2o3_jdat`, `fe2o3_stds`, `fe2o3_syntax`, `fe2o3_text`, `fe2o3_units`, `crossterm`, `secrecy`.

**Current capabilities:**
- `repl` -- read-eval-print loop framework.
- `window` -- terminal window management.
- `draw` -- drawing primitives.
- `render` -- rendering pipeline.
- `event` -- terminal event handling.
- `input` -- user input processing.
- `action` -- action/command dispatch.
- `cmds` -- built-in commands.
- `cfg` -- TUI configuration.
- `style` -- terminal styling (colours, formatting).
- `text` -- text rendering.

**Examples:** `repl.rs` example.

**What is next:**
- Command handling has unimplemented paths (`cmds.rs` line 102).

**Tests:** 2 test files (`draw.rs`, `main.rs`).

---

### Application-Level Crates

These are the highest-level crates, combining many lower-level crates into complete applications or protocols.

#### fe2o3_net -- Networking Utilities

**Purpose:** Networking primitives and protocol implementations.

**Dependencies:** `fe2o3_bot`, `fe2o3_core`, `fe2o3_crypto`, `fe2o3_data`, `fe2o3_jdat`, `fe2o3_hash`, `fe2o3_iop_crypto`, `fe2o3_iop_db`, `fe2o3_iop_hash`, `fe2o3_stds`, `fe2o3_syntax`, `fe2o3_text`, plus `tokio`, `tokio-rustls`, `lettre`, `chrono`, `sha1`, `secrecy`, `strum`.

**Current capabilities:**
- `addr` -- network address handling.
- `dns` -- DNS resolution.
- `http` -- HTTP request/response handling.
- `ws` -- WebSocket implementation.
- `smtp` -- SMTP client.
- `email` -- email construction and sending.
- `charset` -- character set handling.
- `media` -- MIME/media type definitions.
- `conc` -- concurrency utilities for networking.
- `file` -- network file operations.
- `id` -- network identification.
- `time` -- network time utilities.
- `constant` -- networking constants.

**What is next:**
- Media type definitions incomplete (`media.rs` lines 18, 125).
- Header field encapsulation needed (`http.rs` line 1054).
- WebSocket continuation frames not supported (`ws.rs` line 431).

**Tests:** 4 test files (`email.rs`, `http.rs`, `main.rs`, `smtp.rs`).
**README:** Includes port forwarding notes for privileged ports.

---

#### fe2o3_o3db -- Ozone O3DB Database

**Purpose:** A log-structured key-value database inspired by BitCask, using OS threads (bots) for concurrent operations.

**Dependencies:** 15+ internal crates plus `crossbeam-utils`, `hostname`, `humantime`, `lazy_static`, `num_cpus`, `rand`, `regex`, `secrecy`, `seahash`.

**Current capabilities:**
- `O3db<>` -- generic database struct.
- `api` -- public database API (get, put, delete).
- `base` -- base types and configuration.
- `db` -- main database implementation.
- `bots/` -- worker thread system:
  - `bot_config` -- configuration bot.
  - `bot_server` -- server bot.
  - `bot_zone` -- zone management bot.
  - `bot_super` -- supervisor bot.
  - `worker/` -- worker bots for read/write operations.
- `comm` -- inter-bot communication.
- `dal/` -- data abstraction layer.
- `data/` -- core data structures.
- `file/` -- file management:
  - `fcache` -- file caching.
  - `live` -- live file handling.
  - `floc` -- file location tracking.
  - `zdir` -- zone directories.
  - `state` -- file state management.
  - `stored` -- stored file handling.
- `test` -- testing utilities.

**Key features:**
- Log-structured append-only storage.
- Memory-optimised design with configurable cache.
- Automatic garbage collection of stale data.
- Simple key-value interface.
- Multi-zone support for data partitioning.
- Configurable reader/writer thread counts.
- Robust cache initialisation and server lifecycle.
- Timestamped entries.

**What is next (from internal roadmap):**
- Recaching system not yet implemented.
- Rezoning (dynamic zone rebalancing) not yet implemented.
- User access control not yet implemented.
- Documentation needs expanding.
- Extensive testing still required.
- Performance optimisation deferred.
- Value writing optimisation (`bot_writer.rs` line 398).
- GC file status reporting (`bot_file.rs` line 269).
- Delete message handling (`bot_server.rs` line 98).
- Several unfinished items in `server.rs` (lines 419, 452, 469, 478, 517).
- Archive GC unimplemented (`archive/gc.rs` line 160).

**Benchmarks:** 1 benchmark file.
**Tests:** 4 test files (`basic.rs`, `dal.rs`, `main.rs`, `perf.rs`).

---

#### fe2o3_shield -- SHIELD Protocol

**Full name:** Secure Hash In Every Little Datagram.

**Purpose:** A secure peer-to-peer protocol built on UDP, with integrated post-quantum cryptography and proof-of-work based DOS protection.

**Dependencies:** 10+ internal crates plus `lettre`, `local-ip-address`, `num_cpus`, `rand`, `secrecy`, `tokio`.

**Current capabilities:**
- `Shield<>`, `Protocol<>`, `ShieldParams<>` -- core protocol types.
- `server` -- UDP server implementation.
- `packet` -- packet definitions and handling.
- `msg/` -- message handling with syntax definitions.
- `guard` -- access control and user management.
- `pow` -- proof-of-work challenge/response for DOS mitigation.
- `schemes` -- cryptographic scheme negotiation.
- `cfg` -- protocol configuration.
- `constant` -- protocol constants.
- `core` -- core protocol logic.

**Key features:**
- UDP-based secure messaging.
- Post-quantum cryptographic key exchange and signing.
- Proof-of-work challenges to prevent spam/DOS.
- Per-user access control.
- Configurable cryptographic scheme negotiation.

**What is next:**
- Invalid signature handling incomplete (`server.rs` lines 419, 452).
- Periodic garbage collection of inactive users (`server.rs` line 469).
- Several message completion items pending (`server.rs` lines 478, 517).
- Proof-of-work bypass needs review (`constant.rs` line 46).
- Additional validation checks needed (`pow.rs` line 187).
- User log examination needed (`guard/user.rs` line 84).

**Examples:** UDP echo server example.
**Tests:** 2 test files (`main.rs`, `msg.rs`).

---

#### fe2o3_steel -- Secure TCP Server

**Purpose:** A secure TCP server supporting HTTPS, WebSocket and SMTPS (secure SMTP). Designed with no non-secure communication paths.

**Type:** Library + binary (`steel`).

**Dependencies:** 20+ crates including `tokio`, `rustls`, `rcgen`, `crossterm`, `rpassword`, `zeroize`, `swc`, `grass`, plus 13+ internal fe2o3 crates.

**Current capabilities:**
- `srv` -- server implementation:
  - TLS/mTLS support via `rustls`.
  - Self-signed certificate generation via `rcgen`.
  - HTTPS request routing.
  - WebSocket upgrade and handling.
  - SMTPS server.
- `app` -- application layer:
  - `repl.rs` -- interactive REPL for server management.
  - `https.rs` -- HTTPS application logic.
- Asset pipeline:
  - JavaScript bundling via SWC.
  - SASS/CSS compilation via grass.

**What is next:**
- Help object caching (`app/repl.rs` line 178).
- Dynamic route registration (`app/https.rs` line 119).
- Windows certificate testing (`srv/cert.rs` line 328).

**README:** Includes port forwarding notes for privileged ports.
**Tests:** 3 test files (`client.rs`, `main.rs`, `server.rs`).

---

### Procedural Macro Crates

#### new -- Constructor Derivation

**Path:** `fe2o3_core/new`

**Purpose:** Provides `#[derive(New)]` for automatic constructor generation.

**Dependencies:** `syn`, `quote`, `proc-macro2`.

---

#### dat_map -- Map Derivation

**Path:** `fe2o3_jdat/dat_map`

**Purpose:** Provides `#[derive(FromDatMap, ToDatMap)]` for automatic JDAT map serialisation.

**Dependencies:** `fe2o3_core`, `syn`, `quote`, `proc-macro2`.

---

## Cross-Cutting Concerns

### Error Handling

All crates use the `Outcome<V>` type from `fe2o3_core` (a `Result` alias) with custom macros:
- `res!(expr)` -- replaces `?` operator, propagates errors with context.
- `ok!(option, msg)` -- replaces `.unwrap()`, returns error on `None`.
- `err!(msg; Tag1, Tag2)` -- creates tagged errors.
- `catch!(expr, handler)` -- error handling blocks.

### Lock Handling

Safe lock acquisition macros that handle poisoned locks gracefully:
- `lock_read!(rwlock)`, `lock_write!(rwlock)` -- RwLock.
- `lock_mutex!(mutex)`, `lock_mutex_thread!(mutex, context)` -- Mutex.

### Naming and Registration

The `fe2o3_namex` crate provides a universal naming system. Crates that define schemes or algorithms register them via the `InNamex` trait, enabling distributed identification without central coordination.

### Serialisation

JDAT (`fe2o3_jdat`) serves as the primary serialisation format throughout the library, used for configuration, wire protocols, database storage and inter-component communication.

---

## Testing Summary

| Crate | Test Files | Benchmarks |
|---|---|---|
| fe2o3_core | 3 | -- |
| fe2o3_stds | -- | -- |
| fe2o3_geom | 1 | -- |
| fe2o3_test | -- | -- |
| fe2o3_num | 1 | -- |
| fe2o3_text | 5 | -- |
| fe2o3_units | -- | -- |
| fe2o3_bot | -- | -- |
| fe2o3_data | 3 | -- |
| fe2o3_file | 2 | -- |
| fe2o3_jdat | 5 | 2 |
| fe2o3_namex | 3 | -- |
| fe2o3_hash | 3 | -- |
| fe2o3_crypto | 1 | -- |
| fe2o3_syntax | 3 | -- |
| fe2o3_tui | 2 | -- |
| fe2o3_iop_crypto | -- | -- |
| fe2o3_iop_hash | -- | -- |
| fe2o3_iop_db | -- | -- |
| fe2o3_net | 4 | -- |
| fe2o3_o3db | 4 | 1 |
| fe2o3_shield | 2 | -- |
| fe2o3_steel | 3 | -- |
| **Total** | **45** | **3** |

---

## External Dependencies

### Heavy/Notable Dependencies

| Dependency | Used By | Purpose |
|---|---|---|
| tokio | steel, shield, net | Async runtime |
| rustls, tokio-rustls | steel, net | TLS implementation |
| pqcrypto-dilithium | crypto | Post-quantum signatures |
| aes-gcm | crypto | Symmetric encryption |
| ed25519-dalek | crypto | Elliptic curve signatures |
| swc | steel | JavaScript bundling |
| grass | steel | SASS/CSS compilation |
| crossterm | tui | Terminal abstraction |
| lettre | net, shield | Email sending |
| seahash | hash, o3db | Fast hashing |
| tiny-keccak | hash | SHA-3/Keccak |
| rust-argon2 | hash | Password hashing/KDF |
| bigdecimal, num-bigint | num, jdat | Arbitrary precision numbers |
| wasm-bindgen | crypto | WebAssembly support |

---

## Overall Project Status and Roadmap

### Completed

- Core error handling and macro system.
- JDAT format with string and binary encoding/decoding.
- Post-quantum cryptography (Dilithium, SABER, AES-GCM, ED25519).
- Basic O3DB database with log-structured storage and GC.
- SHIELD protocol with UDP messaging and PoW.
- Steel secure server with HTTPS, WebSocket and SMTPS.
- TUI framework with REPL.
- Syntax system bridging CLI and wire protocols.
- IOP abstraction layers for crypto, hashing and databases.

### In Progress / Next Steps

1. **O3DB:** Recaching, rezoning, user access control, expanded testing and documentation.
2. **SHIELD:** Protocol completion -- signature handling, user GC, message handling.
3. **Steel:** Dynamic routing, help caching, Windows support.
4. **Crypto:** Timing-safe comparisons, SABER variant completion, Dilithium optimisation.
5. **Net:** WebSocket continuation frames, media type completion, header encapsulation.
6. **Units:** Scale lookup implementation.
7. **Namex:** Date validation.
8. **General:** API stabilisation, expanded documentation, community contribution readiness, crates.io publication.

### Design Philosophy Reminders

- Readable and obvious over clever.
- Correctness and reliability before optimisation.
- No `unsafe`, no `unwrap()`.
- Minimal third-party dependencies where practical.
- Self-contained implementations built from first principles.
