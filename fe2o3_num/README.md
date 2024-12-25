# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This is a fundamental numerical crate providing support for primitive and arbitrary-precision numbers in Hematite. It includes:

- Unit structs `Float32` and `Float64` that wrap `f32` and `f64` with additional capabilities like total ordering and hashing
- Support for arbitrary-precision integers and decimals via `BigInt` and `BigDecimal` types
- A robust `NumberString` parser that handles various numerical formats including:
  - Base 10 integers and decimals with optional scientific notation
  - Hexadecimal (0x prefix)
  - Octal (0o prefix)
  - Binary (0b prefix)
- Convenience macros for working with arbitrary-precision numbers
- Traits for common numeric operations

The crate aims to provide a consistent interface for working with numbers whilst maintaining type safety and avoiding unsafe code.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
