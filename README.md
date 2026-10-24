# concord-telemetry

One `init`, domain events and coded error records on OpenTelemetry. Binaries
only: library, language-agnostic and wasm/Flutter-native crates never depend on
it. Consumers depend on `concord-telemetry` and `tracing`, never on an
`opentelemetry*` crate directly.

Workspace: `concord-telemetry` (library), `concord-telemetry-derive`
(`#[derive(Event)]`, re-exported), `concord-lint` (checker binary). The shared
field registry is `fields.toml`; `concord-telemetry-proto/proto/concord_telemetry/v1/principal.proto`
is the `Principal` message (hand-written prost types in `concord-telemetry`; the
tiny `concord-telemetry-proto` crate exposes the proto's include path to
consumers' `build.rs`, see "Importing `principal.proto`" below).

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

### Importing `principal.proto` from your build

An event proto with a `Principal` field imports
`concord_telemetry/v1/principal.proto`. Build scripts get its include path from
`concord-telemetry-proto`, which has no dependencies (never put `concord-telemetry`
itself in `[build-dependencies]`: that compiles the OpenTelemetry stack into
your build-script graph):

```toml
[dependencies]
concord-telemetry = { git = "https://github.com/qtpi-bonding/concord-telemetry.git", rev = "<revision>" }
[build-dependencies]
concord-telemetry-proto = { git = "https://github.com/qtpi-bonding/concord-telemetry.git", rev = "<revision>" }
```

```rust
// build.rs
let includes = [PathBuf::from("proto"), concord_telemetry_proto::proto_include_dir()];
let descriptor = PathBuf::from(std::env::var("OUT_DIR")?).join("descriptor.bin");
prost_build::Config::new()   // or tonic_build::configure()
    .file_descriptor_set_path(&descriptor)
    .extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")
    .compile_protos(&["proto/yours/v1/events.proto"], &includes)?;
pbjson_build::Builder::new()
    .register_descriptors(&std::fs::read(&descriptor)?)?
    .extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")
    .build(&[".yours.v1"])?;
```

Both `extern_path` calls are needed: prost for the message, pbjson for serde.
`proto-consumer-test/` is a working copy of this build, run by `scripts/check.sh`.

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
`error_anyhow!(&err, source = X::NAME)` (without `source`, the source is the calling module path, as with `error!`) record `error.code`, `error.class`,
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

`scripts/check-otel-dependents.sh [-p PACKAGE] ALLOWED_CRATE...` is reusable in
your own workspace: it fails if any crate outside the OpenTelemetry family and
the crates you name (normally `concord-telemetry`) depends directly on
`opentelemetry`, using `cargo tree -e normal -i opentelemetry --depth 1`. Without
`-p` it checks the whole workspace. It runs cargo through `$CARGO_SLOT` when set.
Copy it or call it from a pinned checkout; do not rewrite it in your check script.

`scripts/check.sh` (through the compile slots): fmt, clippy, tests, the
no-default-features build, the opentelemetry dependents allowlist (`scripts/check-otel-dependents.sh`), no tonic, no
rustls in the default tree, and the lint on this repo. `collector/otel-collector.yaml`
is an example collector config for the end-to-end stack.
