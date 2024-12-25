# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This is a fundamental crate providing interoperability traits for hashing, checksums and key derivation functions within the Hematite ecosystem. It defines core interfaces that enable implementations of various hashing schemes to work together:

- `Hasher` - For general-purpose hash functions that produce fixed-size outputs
- `Checksummer` - For calculating and verifying checksums over arbitrary data
- `KeyDeriver` - For key derivation functions used in password hashing and verification

The traits are designed to be implementation-agnostic while ensuring thread safety and proper error handling through Hematite's `Outcome` type.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
