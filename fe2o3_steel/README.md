# Oxedize Hematite (fe2o3)

Hematite is a collection of crates built from first principles, providing foundational tools for building robust applications.  See the [github repo](https://github.com/Oxedize/fe2o3) for more information.

## This crate

Steel is Hematite's TCP server implementation providing HTTPS, WebSocket and SMTPS support. It includes:

- A robust HTTPS server with WebSocket upgrade support
- Development mode with hot reloading and self-signed certificates 
- Production mode with Let's Encrypt certificate automation
- Built-in static file serving and route configuration
- Clean separation between server and application layers
- JavaScript/TypeScript bundling and SASS compilation in development mode

## Linux iptables and Port Forwarding

When you want to server to the internet using Linux, ports below 1024 are considered privileged, and only the root user can bind to them by default. This is a security feature to prevent unauthorized users from binding to well-known ports.

If you prefer not to grant additional capabilities to your application, another approach is to use iptables to forward traffic from port 443 to a higher, non-privileged port that your application can bind to.

1. Run your application on an unprivileged port (e.g., 8443).

2. Set up iptables to forward traffic from port 443 to the port your application is using bash:

    sudo iptables -t nat -A PREROUTING -p tcp --dport 443 -j REDIRECT --to-ports 8443

This approach doesn't require giving special permissions to your application, as the port forwarding is handled by the kernel.

## Supporting Development

This project is developed and maintained through GitHub Sponsors. If you find Hematite valuable for your work or interesting as a project, consider supporting its continued development. No special perks or privileges - just sustainable open source development of innovative Rust infrastructure and apps.

![GitHub Sponsors](https://img.shields.io/github/sponsors/Oxedize)

## License

See the [LICENSE](LICENSE) file for license rights and limitations.

## Contact

<hello@oxedize.com>
