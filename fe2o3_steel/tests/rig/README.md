# The rig

Starts a real Steel -- its own wallet, its own Ozone, its own certificates -- and
drives it over HTTPS with a real dashboard session. Then throws it all away.

`cargo test` does not run this. It is a shell script, it wants a free port and
half a minute, and it is here to be run by hand:

```
fe2o3_steel/tests/rig/run.sh
```

Nothing else is needed. It builds the binary, makes a wallet, starts a server in
a temporary directory, runs the checks, and stops.

## Why it exists

An in-crate test asks the code whether it works. The code says yes. That is worth
something, and it is not worth what it looks like: a DKIM signer in this workspace
sat broken for three months behind tests that agreed with it.

Some things cannot be asked from inside at all. Whether the dashboard's cookie
reaches a handler is a fact about `Path=/admin` and a browser, not about a
function. The composer's whole design rests on it, and the route it replaced was
gated on a session that could never arrive -- a gate that refused its own author
exactly as it refused a stranger, and therefore looked correct from every angle
except this one.

So the checks here are the ones only an outside caller can make: that an
anonymous write is refused, that a draft is served to nobody while its author can
still read it, that a renamed post takes its old key with it, that a deleted post
stays deleted through an import, and that a slug with a path in it writes nothing.

## What it is not

It is not a substitute for the unit tests, and it does not run in CI. It is slow,
it binds a port, and it will fail on a machine where port 9443 is busy
(`RIG_PORT=nnnn` moves it).

It knows some things that cost an afternoon to learn, and they are in `run.sh`
where they bite:

- **Wallet creation needs a terminal.** It reads the passphrase through
  crossterm's raw mode, so a pipe will not do -- hence the pty in
  `make_wallet.py`. In raw mode Enter is a carriage return, not a newline.
- **`server -d` or nothing.** Without `-d` a first run refuses to start in
  production mode and exits 0, silently, having said nothing about why.
- **stdin must stay open.** The server keeps a shell beside the listener, and an
  EOF on stdin ends the process. Hence the fifo.
- **`dev_cfg` is a required config block.** Every `FromDatMap` field is, unless it
  says otherwise.
- **Run one rig at a time.** A second run while another is held (`RIG_HOLD=1`)
  produces false failures in `console_rig`, which authenticates over a WebSocket
  and will find the wrong server. It looks exactly like a gate regression — a
  non-admin getting 200 instead of 403 — and is not one. Kill the held rig first.

## The passphrase

`rig-test-passphrase-not-a-secret`, in the clear, on purpose. It protects a
wallet that exists for thirty seconds and holds nothing.
