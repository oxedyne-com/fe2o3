# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This crate provides generic hashing functionality, including checksums, cryptographic hashes, and proof-of-work mechanisms. Key features include:

- Support for various hash algorithms including SHA3-256 and SeaHash
- CRC32 checksum implementation with verification
- Key derivation using the Argon2 password hashing algorithm
- A sharded hashmap implementation for concurrent access
- Proof-of-work system with configurable difficulty and multi-threaded mining
- Integration with the Namex universal name codex for algorithm identification
- Generic traits allowing custom hash algorithm implementations

The crate aims to provide a flexible foundation for applications requiring secure hashing, data verification, and proof-of-work systems while maintaining type safety and error handling through Hematite's core error types.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
