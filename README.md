# Oxedize Hematite (fe2o3)

Hematite is a collection of Rust crates that grew from an exploration into database design and key data structures.  The project includes several innovative components:

- **O3db**: A log-structured key-value database inspired by BitCask, designed for high throughput.
- **JDAT**: A superset of JSON adding types, combining human-readable and binary formats.
- **Shield**: A secure peer-to-peer protocol and app built on UDP.
- **Namex**: A distributed universal name codex for schemes and specifications.
- **Steel**: A TCP server implementation with HTTP, HTTPS, WebSocket, and SMTPS support.
- **Ironic**: A terminal user interface.

Oxedize is my Rust/Web development shop.

## Status

This solo project has been in progress on a part time basis for several years, and I estimate completion of the initial 0.9.0 release is at around 50% overall.  I'm now doing this in public in the hope of soliciting ideas and suggestions, with a view to publishing a developer guide and accepting contributions some time in the next 12 months.
 
Feel free to:
- Star/watch the repository if interested,
- Open issues for bugs or feature suggestions,
- Fork and experiment with the code,
- Provide feedback through discussions,
- Contribute through GitHub Sponsors if you want faster development or wider adoption, and
- Make pull requests once the contribution window opens.

See the detailed [project progress](PROGRESS.md) for recently completed and upcoming features.

## Project Philosophy

Hematite started as a Rust learning experience, and one of my objectives has been to minimise third-party dependencies. However, despite every effort to reinvent lots of wheels, it still depends on many crates directly and indirectly.  So I am very grateful to all their authors and contributors.

The code aims mostly to be correct, readable, and maintainable rather than super-fast or clever.  While I have always strived for economy and efficiency, I'm looking forward to community contributions that help make better of Rust features, and lead to more aggressive optimisation.

One of the initial motivations for the library was to improve my personal developer experience, which has involved a range of experiments particularly around error handling and logging.  The code avoids the use of `unsafe` and `unwrap`.  The `?` propogator is avoided in favour of macros that are more explicitly function-like such as the match-based `ok!` and the closure `res!` which tries to also capture a class of panics.  Eventually any impacts on performance will be properly assessed.  Tags were built into the `fe2o3_core::error::Error` type which offers a foundation for theoretical benefits but the practical value remains to be seen.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## Components

The crates can be organised by their level of internal cross-dependencies, currently the most notable being:

### Foundational (no internal cross-dependencies)
- **fe2o3_core**: Core traits, utilities, logging, and fundamental types, typically small and relatively simple.
- **fe2o3_stds**: A place for existing public standards.

### Fundamental (1-3 internal cross-dependencies)
- **fe2o3_bot**: Thread worker library, for bot-like worker threads with common termination semantics.
- **fe2o3_data**: Specialised data structures such as ring buffers, stacks, trees and timestamped types.
- **fe2o3_iop_hash**: Interoperability layer for hashing, including SHA3-256 and Seahash.
- **fe2o3_net**: Network utilities with support for DNS, HTTP, WebSocket, and SMTP.
- **fe2o3_num**: Numerical type utilities, big integer, decimal types, and number strings.
- **fe2o3_text**: String manipulation and formatting, with text processing tools, base2x encoding and a REPL-friendly Stringer.

### Functional (4-7 internal cross-dependencies)
- **fe2o3_crypto**: Cryptography library including post-quantum implementations such as SABER and Dilithium.
- **fe2o3_hash**: Generic hashing and key derivation utilities.
- **fe2o3_iop_crypto**: Interoperability layer for cryptography, including AES-GCM, and Ed25519.
- **fe2o3_iop_db**: Interoperability layer for databases.
- **fe2o3_jdat**: A user-level type layer including JDAT format implementation.
- **fe2o3_namex**: A distributed, general purpose, universal name codex with utilities.
- **fe2o3_syntax**: A command parsing library for defining custom protocols.

### Application Level (8+ internal cross-dependencies)
- **fe2o3_o3db**: The Ozone database.
- **fe2o3_shield**: Shield protocol implementation for secure peer-to-peer networking.
- **fe2o3_steel**: A web server implementation with developer mode, HTTPS, WebSocket, and SMTPS support.
- **fe2o3_tui**: A terminal user interface library, including the Ironic TUI.

## Getting Started

Hematite is currently available via [GitHub]("https://github.com/Oxedize/fe2o3.git") and version 0.9.0 will be available on [crates.io](crates.io). You can use it in several ways:

### From GitHub

For the latest development version, use git dependencies:
```toml
[dependencies]
oxedize_fe2o3 = { git = "https://github.com/Oxedize/fe2o3" }
```

### Local Development

To explore or contribute:

1. Clone the repository:
```bash
git clone https://github.com/Oxedize/fe2o3.git
cd fe2o3
```

2. Build the project:
```bash
cargo build
```

3. Run the tests:
```bash
cargo test
```

Tests are grouped inside files within a `tests` directory for each crate, and run via the `tests/main.rs` file, e.g.:
```bash
cd fe2o3_core
cargo test main -- --nocapture
```

### From crates.io

When it becomes available, you can access specific crates directly :
```toml
[dependencies]
oxedize_fe2o3_core = "0.5.0"
oxedize_fe2o3_jdat = "0.5.0"
```

Or use the workspace crate to access everything:
```toml
[dependencies]
oxedize_fe2o3 = { version = "0.5.0", features = ["all"] }
```

Or just the components you need:
```toml
[dependencies]
oxedize_fe2o3 = { version = "0.5.0", features = ["core", "jdat", "o3db"] }
```

### Usage Examples

Check the test files in each crate's `tests` directory for detailed usage examples. Here's a quick start:

```rust
// Using the workspace crate
use oxedize_fe2o3::core::prelude::*;
use oxedize_fe2o3::jdat;

// Or individual crates
use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat;
```

## Highlights

### Ozone Database
O3db aims to:
- Write data quickly to log-structured files,
- Offer a high degree of parallalism using operating system threads,
- Keep as much data in a fast volatile cache as possible,
- Provide automatic file garbage collection, and a
- Simple, familiar key-value interface.

### JDAT
JDAT extends JSON to support:
- Type annotations including multiple number formats,
- Binary encoding and decoding,
- Comment support, and
- Any type as map keys.

### Shield Protocol
Shield stands for Signed Hash In Every Little Datagram and provides:
- UDP-based messaging,
- Secure and authenticated peer-to-peer communication,
- Post-quantum cryptography options, and
- Mitigates DOS attacks through use of proof of work in all packets.

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Future Plans

- Complete initial implementation of all components,
- Stabilise APIs,
- Expand documentation,
- Open for community contributions,
- Publish to crates.io.

## Contact

<hello@oxedize.com>
