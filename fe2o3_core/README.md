# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This is a foundational crate providing core functionality used throughout the Hematite ecosystem:

- Rich error handling with tags and error chaining via `Outcome<T>` and `Error<T>`
- A flexible logging system with console and file output support
- Thread and bot management with message passing primitives
- Generic data structures and traits including maps, counters, and alternates
- Numeric type utilities and bounds checking
- String and path manipulation helpers
- Byte handling and conversion traits
- Testing utilities with filtering and assertion support
- A derive macro for generating `new()` constructors

Core types include `Outcome<T>` for error handling, `Logger` for logging, `Counter` for numeric iteration, and `SimplexThread` for thread management. The error system allows tagging errors with multiple categories and chaining them together while preserving context through propagation.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
