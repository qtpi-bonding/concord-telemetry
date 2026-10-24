# concord-telemetry

One `init`, domain events and coded error records on OpenTelemetry. Binaries
only: library, language-agnostic and wasm/Flutter-native crates never depend on
it. Consumers depend on `concord-telemetry` and `tracing`, never on an
`opentelemetry*` crate directly.

Workspace: `concord-telemetry` (library), `concord-telemetry-derive`
(`#[derive(Event)]`, re-exported), `concord-lint` (checker binary). The shared
field registry is `fields.toml`; `proto/concord_telemetry/v1/principal.proto`
is the `Principal` message (hand-written prost types, no build step).

## Init

```rust
let _guard = concord_telemetry::init(concord_telemetry::Config::new("grorg-server", env!("CARGO_PKG_VERSION")))?;
```

Keep the `Guard` for the life of `main`: dropping it flushes (capped by
`Config::flush_deadline`, default 2s). No exporter is built unless
`OTEL_EXPORTER_OTLP_ENDPOINT` is set (a base URL, plain HTTP; the exporter
appends `/v1/logs` and `/v1/traces`). Local output goes to stderr (`Text`,
`Json`) or a file (`JsonFile`), never stdout. Default filter is `info` when
`RUST_LOG` is unset; events and error records (target
`concord_telemetry::event`) are always enabled, so they also print locally
under `RUST_LOG=error`. Do not call `std::process::exit`: it skips the flush
(`concord-lint` flags it in binaries).

Plain HTTP by default with no TLS stack in the tree. HTTPS to the collector is
behind the optional `tls` feature (reqwest's rustls, aws-lc-rs). TLS is an ops
concern; prefer a tailnet or a Caddy in front.

## Events

```rust
concord_telemetry::event!(ActionStatusChanged { source_record_id: id.clone(), /* ... */ });
```

Generate event messages with prost and add the derive from `build.rs`, derive
line first and helper second, with `message_attribute` (never `type_attribute`,
which also reaches enums and oneofs):

```rust
.message_attribute(".grorg.events", "#[derive(::concord_telemetry::Event)]")
.message_attribute(".grorg.events", "#[concord_event(package = \"grorg.events.v1\")]")
.extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")
```

The derive needs `as_str_name`/`from_str_name` on enums, which prost-build
emits. Rules: scalars, strings, enums (by name), `Timestamp`/`Duration`
(`prost_types`, `pbjson_types`, `identity_crypto`; rendered with `Z`),
repeated scalars (one JSON-array string attribute), `Principal`, and a required
`source_record_id: String`. Nested messages, `bytes`, `map` and `oneof` are
rejected, as are never-log names. Attribute names are the proto field names
(snake_case). `u64` is recorded as a decimal string (OTel has no unsigned
attribute); an unrecognised enum number is emitted as `UNRECOGNIZED` and does
not round-trip (`from_attributes` returns `RebuildError::Parse`).
`Principal` attributes are `<field>.kind`, `.id`, `.verified_by` (JSON array
string), `.via`; its serde JSON uses `verifiedBy` (pbjson-compatible).

## Error records

`concord_telemetry::error!(&err, rpc.method = %m)` and
`error_anyhow!(&err, source = X::NAME)` record `error.code`, `error.class`,
`error.fingerprint`, `error.source` and your fields, never the error's Display
text. A panic is recorded as `concord.process.panic` (class unknown, source the
panic location).

## Testing

Enable the `testing` feature in dev-dependencies. `testing::capture()` installs
one process-global subscriber (panics if one is already set) and returns a
`Capture`; find events by a domain value you created:
`cap.single_event_where::<E>(|e| e.source_record_id == id)`. Tests never call
`init`. `capture_exclusive()` serializes and clears the store (use it in a test
binary of its own). If your tests already install a global subscriber, add
`testing::layer()` to it and read with `testing::reader()`. Both paths were
confirmed against grorg (tonic server tests) and structmux (daemon tests).

## concord-lint

Install at the rev you pin: `cargo install --git https://github.com/qtpi-bonding/concord-telemetry --rev <rev> concord-lint`.
`concord-lint [--local fields.local.toml] [--baseline lint-baseline.txt [--write-baseline]] <paths...>`
checks tracing event macros (messages are fixed strings, no interpolation or
arguments), field names against `fields.toml` plus your local overlay, never-log
names everywhere, and `process::exit` in binaries. Exit 0 clean, 1 violations,
2 usage or parse error. The baseline counts repeats, so a new copy of a
baselined violation is new. It parses source and never compiles your crate.
Known limits: span macros and `#[instrument(fields(..))]` are not checked;
`process::exit` in modules of a binary (`mod cli;`) and a bare `exit(1)` after
`use std::process::exit` are not detected.

## Checks

`scripts/check.sh` (through the compile slots): fmt, clippy, tests, the
no-default-features build, the opentelemetry dependents allowlist, no tonic, no
rustls in the default tree, and the lint on this repo. `collector/otel-collector.yaml`
is an example collector config for the end-to-end stack.
