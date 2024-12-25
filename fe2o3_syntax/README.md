# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

This crate provides a protocol-oriented syntax system that unifies command handling for both textual REPL interfaces and network communications. It allows you to define a `Syntax` that specifies commands, arguments, and expected values in a structured way, while maintaining direct control over command handling. The library deliberately avoids callback mechanisms in favour of explicit command matching, promoting simplicity and transparency in command processing flows.

Key features:
- Define commands with strongly-typed arguments and values
- Parse both text and binary message formats
- Support for short and long-form argument styles
- Built-in help text generation
- Seamless operation across local REPL and network contexts

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
