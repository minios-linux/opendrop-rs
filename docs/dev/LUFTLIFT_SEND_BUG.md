# Task: fix luftlift send to link-local IPv6 (scope_id / EINVAL)

A `luftlift -> luftlift` round-trip test exposed a real bug in the **send** client.
It is NOT a protocol-faithfulness issue — it's a socket-addressing detail the audit
did not cover.

## Reproduce
- Run a receiver: `luftlift -i awdl0 receive -o /tmp/ldrop` (announces e.g.
  `luftlift-<pid>`).
- Send to it: `luftlift -i awdl0 send luftlift-<pid> /tmp/somefile`.

## Observed
```
INFO discovered AirDrop peer name=luftlift-... addresses={fe80::a8bb:ccff:fedd:ee01}
INFO receiver found addr=fe80::a8bb:ccff:fedd:ee01 port=8771
INFO client: POST /Discover
Error: Invalid argument (os error 22)
```

## Root cause
The receiver's address is an IPv6 **link-local** address (`fe80::/10`). Connecting to a
link-local address REQUIRES a **scope_id** (the interface zone, i.e. `%awdl0`). The send
client builds the `SocketAddrV6` with `scope_id = 0`, so `TcpStream::connect` /
`rustls` fails with `EINVAL` (os error 22).

## Fix
When the resolved peer address is IPv6 link-local (`is_unicast_link_local()` /
`fe80::/10`) and its `scope_id` is 0, set the scope_id to the interface index of the
`-i <interface>` option (e.g. `awdl0`). Resolve the interface name to an index with
`libc::if_nametoindex` (a crate-local helper) and build
`SocketAddrV6::new(addr, port, 0, scope_id)`. Apply this in the send client's connect
path (wherever it builds the SocketAddr to connect the TLS/TCP stream).

mdns-sd discovery returns the address but not the awdl0 scope, so the client must attach
it from the configured interface. (The receiver binds fine because it listens on `::`.)

## Deliverable
- Implement the fix in `crates/luftlift-rs/`. TDD where practical (a unit test that the
  connect-addr for a link-local peer carries the right scope_id given an interface index;
  keep the `if_nametoindex` call out of the unit-tested pure function).
- Keep `cargo test -p luftlift-rs` green and `cargo clippy -p luftlift-rs --all-targets
  -- -D warnings` clean. Commit as `fix(luftlift): set IPv6 scope_id for link-local send`.
  Rebuild release.
- After it builds, the `luftlift -> luftlift` round-trip above should complete:
  POST /Discover -> /Ask -> /Upload and the file lands in the receiver's output dir.

Only edit `crates/luftlift-rs/`. Work autonomously.
