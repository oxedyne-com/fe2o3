# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This crate provides a log-structured key-value database inspired by BitCask, designed for high throughput and reliability. The database aims to:

- Write data quickly by appending to log-structured operating system files
- Leverage operating system threads extensively for high parallelism
- Maintain as much data as possible in a fast volatile cache
- Provide automatic file garbage collection
- Offer a simple, familiar key-value interface with robust error handling

Key features include:
- Automatic chunking of large values
- Support for encryption including post-quantum options
- Built-in checksum verification
- Configurable garbage collection
- Comprehensive error handling with error tags and chaining

The crate is part of the Hematite collection but can be used independently for applications requiring a fast, reliable key-value store with strong correctness guarantees.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
