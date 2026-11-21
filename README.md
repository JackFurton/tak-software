# tak-software

A from-scratch TAK (Team Awareness Kit) stack for learning how ATAK, WinTAK and TAK Server talk
to each other, built to interoperate with the real clients.

Everything in TAK is a Cursor on Target (CoT) event: a uid, a type such as `a-f-G-U-C`
(friendly ground combat unit), a point, a validity window, and a `<detail>` bag. Clients send
them as XML or TAK protocol v1 protobuf, over UDP multicast (`239.2.3.1:6969`) or TCP/TLS to a
server (`8087` plain, `8089` TLS).

## Layout

- `crates/cot`: CoT event model, XML and protobuf codecs, mesh and stream framing, type-code
  parsing and MIL-STD-2525C symbol IDs.

## Roadmap

1. CoT library (`crates/cot`)
2. Simulated devices that walk routes and report position
3. Web UI with a live map, 2525 symbols, chat and markers
4. Interop with FreeTAKServer / OpenTAKServer and real ATAK-CIV

## Development

```sh
cargo test
cargo clippy --all-targets
```

Protobuf field tags follow ATAK-CIV's
`commoncommo/core/impl/protobuf/*.proto`, including its rules for which `<detail>` elements may
be promoted to typed fields (see `typed()` in `crates/cot/src/xml.rs`).
