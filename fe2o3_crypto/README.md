# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This crate provides cryptographic primitives with a focus on post-quantum cryptography. It includes implementations of:

- The SABER post-quantum key encapsulation mechanism, including LightSaber, Saber and FireSaber variants
- The Dilithium post-quantum digital signature scheme
- AES-256-GCM symmetric encryption
- Ed25519 digital signatures
- Generic traits and types for encryption, signing and key management

The implementations aim to be memory safe and avoid panics while maintaining efficiency. Post-quantum schemes like SABER and Dilithium are implemented based on reference implementations and test vectors to ensure correctness.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
