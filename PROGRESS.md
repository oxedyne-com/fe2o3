# Hematite Progress

This is a dynamic document, and shows the latest goals of development work.

## Jdat functionality: `fe2o3_jdat`

- [x] Added implicit tuple string decoding for round brackets, e.g. explicit (tup2|[1,2]), implicit (1,2)

## Data functionality: `fe2o3_data`

- [ ] Generic `UndoManager` to provide multi-user support for any kind of product/state

## Ozone database: `fe2o3_o3db`

- [x] Basic functional database with (k, v) create, read, update and delete (CRUD)
- [ ] Reliably functional garbage collection
- [ ] Establish query functionality
    - [ ] Basic key pattern matching, e.g. `"user/*/profile"`
    - [ ] Regex key pattern matching, e.g. `"user\/\d+\/name"`
    - [ ] Query api using `fe2o3_syntax`, e.g.
    ```
    query --key (or|["user/A*", "user/B*"]) --map-key (regex|"[aA]ge") --map-val (range|(20.0, 30.0)) --lim 10
    ```
        - [ ] Custom kinds
        - [ ] Boolean logic trees
        - [ ] Regex
        - [ ] Numerical ranges
    - [ ] Query results caching
    - [ ] Employ the `UndoManager` for atomic transactions
    - [ ] Query result streaming for large datasets

## Network functionality: `fe2o3_net`

- [ ] Generic `AddressGuard` to provide protection against threatening network requests from addresses
- [ ] Generic `UserGuard` to provide protection against threatening network requests from users

## Steel web server: `fe2o3_steel`

- [x] Generic TCP server and HTTP, HTTPS and HTML library foundations in `fe2o3_net`
- [x] Separation of app (`./src/main.rs`, `./src/app/`) and library (`./src/lib.rs`, `./src/srv/`) structure 
- [x] Basic TLS certificate management
- [x] Basic functional HTTPS server providing only GET support
- [x] Functional websocket upgrade and javascript interaction
- [x] Integration of database with server and websockets
- [x] Working HTTPS server dev mode with local browser live refresh and default www tree
- [ ] Generic SMTP, SMTPS and email library foundations in `fe2o3_net`
- [ ] Basic functional SMTPS server with database interactivity
- [ ] Expand HTTPS server functionality to all request types
- [ ] Basic hardening of HTTPS server production mode with:
    - [ ] More advanced HTTPS/TLS configuration
    - [ ] Cross-Origin Resource Sharing (CORS) controls
    - [ ] Integration of `AddressGuard` and `UserGuard`, e.g. rate limiting and other protections
- [ ] Integration of email functionality with web server e.g. login via email code
- [ ] Demonstrate a stable production HTTPS website created using Steel dev mode 

## Shield protocol and app: `fe2o3_shield`

- [ ] Split code into app and server like `fe2o3_steel`
- [ ] Complete handshake functionality
- [ ] Create tests for session message sequences
- [ ] Integration of database with server
- [ ] Create an `fe2o3_syntax` for network messages
- [ ] Demonstrate a small peer to peer network on the open internet including discovery

## Ironic app: `fe2o3_tui`

- [ ] Split code into app and library like `fe2o3_steel`, move app out of ./examples
- ...
- [ ] Demonstrate development of Hematite in Ironic as a vim replacement
