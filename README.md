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
- `crates/taksim`: simulated devices that walk routes and report position, plus a mesh listener.
- `crates/takserver`: relays CoT between TAK clients (TCP 8087, TLS 8089), the mesh, and the
  web UI. Negotiates protobuf per connection and honours `<marti><dest>` addressing.
- `web/`: browser client (React, MapLibre, milsymbol) that acts as a TAK device: map, contacts,
  GeoChat including direct messages, markers.
- `scenarios/`: unit definitions for `taksim run --scenario`.

## Running it

```sh
(cd web && npm install && npm run build)
cargo run -p takserver                                   # TAK on :8087, UI on http://127.0.0.1:8080
cargo run -p taksim -- run --tcp 127.0.0.1:8087          # in another terminal
```

Add `--mesh` to takserver to bridge phones that use mesh SA without a server. To point ATAK at
it, add a server connection to this machine's LAN IP, port 8087, protocol TCP. Port 8087 is
unauthenticated, as on TAK Server, so keep it on networks you trust.

### TLS and ATAK data packages

```sh
cargo run -p takserver -- certs init --host 192.168.1.155            # CA + server cert in certs/
cargo run -p takserver -- certs client viper --connect 192.168.1.155:8089
cargo run -p takserver -- --tls 0.0.0.0:8089
```

`certs client` writes `certs/clients/viper.zip`, a data package holding the CA truststore, the
device's certificate and the server entry. Copy it to the phone and open it with ATAK's Import
Manager; the keystore password is `atakatak`. Only devices holding a certificate from this CA can
connect on 8089. Keys are ECDSA P-256 and the .p12 files use the legacy 3DES format Android
expects.

For UI work, `npm run dev` in `web/` serves with hot reload and proxies to the takserver on :8080.

## Simulator only

```sh
cargo run -p taksim -- listen                  # watch the mesh, in one terminal
cargo run -p taksim -- run --mesh              # built-in demo around the National Mall
cargo run -p taksim -- run --stdout --swarm 1  # print raw CoT XML
cargo run -p taksim -- run --tcp 127.0.0.1:8087 --swarm 500   # one connection per unit
```

An ATAK phone on the same network shows mesh units without any server. If it doesn't, pass
`--iface <this machine's LAN IP>` so multicast leaves on the right interface.

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
