# concord-telemetry

## concord-lint ratchet rules

The lint tool reports `ratchet.result_string`, `ratchet.dropped_result`,
`ratchet.error_to_string`, `ratchet.untraced_spawn`, and `ratchet.raw_channel`.
These findings use the same counting baseline mechanism as the existing rules;
test files and `#[cfg(test)]` modules are skipped.

One `init`, domain events and coded error records on OpenTelemetry. A repo's
binaries and its own internal crates (e.g. grorg's `ops`, which emits domain
events after the write) may depend on it. Crates other repos depend on (proto
and type crates), language-agnostic crates and wasm/Flutter-native crates never
do. Consumers depend on `concord-telemetry` and `tracing`, never on an
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
under `RUST_LOG=error`. While exporting, the exporter's own crates are off in
every output, local included, so an export never logs itself into the next
one. The match is by target prefix (`hyper`, `h2`, `reqwest`, `rustls`,
`opentelemetry`), so `hyper_util` and `opentelemetry_sdk` are off too. A more
specific `RUST_LOG` directive, such as `hyper::client=debug`, overrides this.
Do not call `std::process::exit`: it skips the flush
(`concord-lint` flags it in binaries).

Plain HTTP by default with no TLS stack in the tree. HTTPS to the collector is
behind the optional `tls` feature (reqwest's rustls, aws-lc-rs). TLS is an ops
concern; prefer a tailnet or a Caddy in front.

### Resource attributes

Every exported log record and span carries the resource `service.name` and
`service.version` (from `Config`), a generated `service.instance.id`, `host.name`
(the machine hostname) and, when `Config::principal` is set, `principal.kind`,
`principal.id`, `principal.verified_by` and `principal.via`.

`OTEL_RESOURCE_ATTRIBUTES` (`k1=v1,k2=v2`, values percent-encoded) is merged in.
Its `service.version` replaces `Config::service_version`, so a deployment can
set the git commit without a code change, and its `host.name` replaces the
detected hostname. `service.name`, `service.instance.id` and `principal.*` in
it are ignored, and `OTEL_SERVICE_NAME` is not read: the service's identity is
the caller's. A malformed entry (no `=`, empty key, bad percent escape) is
skipped and the others still apply.

Log records also carry `code.file.path`, `code.line.number` and
`code.module.name`, the same keys spans carry. For `error!` and
`error_anyhow!` they are the macro call site; for `event!` too (not the event
type's definition). An event field of the same name wins.

## Trace-aware boundaries

The optional `channel` feature provides `channel::traced` and
`channel::spawn_traced`, which carry the current tracing span (including its
baggage) across Tokio channels and spawned tasks. The optional `http-server`
feature provides `http_server::IngressLayer` for joining W3C HTTP ingress
headers in an `http.server` span.

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

## Trace context

One request keeps one trace id across processes (W3C `traceparent`). With the
`grpc` feature, which adds tower layers over `http::Request` and no tonic:

```rust
Server::builder().layer(concord_telemetry::grpc::ServerLayer) // joins the caller's trace
FooClient::new(concord_telemetry::grpc::client(channel))       // sends it
```

The server span covers a handler up to its response. Items of a streaming
response are produced outside it and carry no trace id. For other carriers,
such as NATS headers, `concord_telemetry::propagation` has `inject`,
`set_parent` and `traceparent`. Hooks and CLIs pass `TRACEPARENT` and
`BAGGAGE` (`set_parent_from_env`); `tracestate` is not carried that way. A crate that
spawns processes but must stay free of OpenTelemetry takes the variable's
name from `concord_telemetry_proto::TRACEPARENT_ENV`. Without an
exporter there are no trace ids and nothing is sent. The `grpc.server` and
`grpc.client` spans are still created, and stay enabled whatever `RUST_LOG`
says, including a directive naming `concord_telemetry::grpc` itself. A server
joins any valid `traceparent` it receives, so an untrusted caller chooses the
trace id and the sampled flag of its own requests.

### Carriers and baggage

`concord_telemetry::carrier` moves a trace across a boundary as a
`TraceCarrier` (the three header values; `proto::v1::TraceCarrier` is its
protobuf form): `capture()` takes the current span's, `parent_on` joins it,
`link_from` starts a new trace linked to it, and `root(name)` starts one with
no parent. `extract_required(hop, ..)` is `parent_on` for a hop that must
carry a trace: with an exporter and no valid `traceparent` it records
`concord.trace.missing` with a `hop` field. `extract_ingress(name, ..)` is
silent on an empty carrier and sets the span attribute `ingress`; the gRPC
`ServerLayer` is an ingress.

`context::add("action_id", id)` puts a label in the current span's baggage:
children and outgoing carriers inherit it, and it is a span attribute.
Baggage is labels only, never authorization or routing. At most 8 entries,
keys up to 64 bytes, values up to 128, the header up to 1024 and `tracestate`
up to 256. Over a cap, or malformed, an entry is dropped and the header's
drops are recorded once as `concord.trace.baggage_dropped` with `dropped`;
the request proceeds. Without an exporter every one of these is a no-op,
`concord.trace.missing` never fires, and local output is unchanged.

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
