# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This is a fundamental crate providing specialised data structures for the Hematite ecosystem:

- `RingBuffer`: A fixed-size circular buffer that maintains a current position and next position pointer
- `RingTimer`: A specialised ring buffer for timing measurements using SystemTime
- `Stack`: An immutable stack implementation with Arc-based nodes for safe concurrent access
- `Tree`: A generic tree structure supporting hierarchical data with focus tracking and display capabilities
- `Timestamped`: A wrapper that associates data with timestamps

These data structures emphasise safety and correctness while providing ergonomic APIs for common use cases.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
