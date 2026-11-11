# concord-telemetry

One `init`, domain events and coded error records on OpenTelemetry. A repo's
binaries and its own internal crates (e.g. grorg's `ops`, which emits domain
events after the write) may depend on it. Crates other repos depend on (proto
and type crates), language-agnostic crates and wasm/Flutter-native crates never
do. Consumers depend on `concord-telemetry` and `tracing`, never on an
`opentelemetry*` crate directly.

Workspace: `concord-telemetry` (library), `concord-telemetry-derive`
(`#[derive(Event)]`, re-exported), `concord-lint` (checker binary),
`concord-fields` (the one parser of `fields.toml`, used by the derive and the lint). The shared
field registry is `fields.toml`. The proto types (`Principal`, `PrincipalKind`,
`Verification`, the wire `TraceCarrier`), `principal.proto`, `trace_carrier.proto`
and the build-script helpers live in the separate
[`concord-telemetry-proto`](https://github.com/qtpi-bonding/concord-telemetry-proto)
repo; `concord-telemetry` depends on it and re-exports the types.

## Breaking changes in 0.4.0

- The proto types moved to `concord-telemetry-proto` (its own repo). The crate
  `concord-telemetry-proto` is no longer in this workspace, so a `[build-dependencies]`
  entry for it points at the new repo, not this one.
- `concord_telemetry::proto::v1` is removed. There is one path per type:
  `concord_telemetry::{Principal, PrincipalKind, Verification, TraceCarrier}` are
  re-exports of `concord_telemetry_proto`'s, and the generated-code path is
  `::concord_telemetry_proto::v1` (`concord_telemetry_proto::RUST_EXTERN_PATH`).
  Change `.extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")`
  to `.extern_path(".concord_telemetry.v1", concord_telemetry_proto::RUST_EXTERN_PATH)`.
- The plain `TraceCarrier` (`concord_telemetry_proto::TraceCarrier`, re-exported)
  and the prost wire message (`concord_telemetry_proto::v1::TraceCarrier`) are
  still two different types. The `From` impls between them and the `field`
  helper moved to `concord-telemetry-proto` (`src/v1.rs`), so they are no longer
  in `concord-telemetry`; the conversions work unchanged where both are in scope.
- `#[derive(Event)]` accepts a `Principal` field typed
  `concord_telemetry_proto::v1::Principal` (and, as before, `concord_telemetry::Principal`).
- `emit_at` is public (`concord_telemetry::emit_at`; it was `__private::emit_at`),
  and `error_at!` / `error_at_anyhow!` are new public macros built on it.
  `emit_error_at(file, line, module, &err, &[(&'static str, AttrValue)])` records
  an error with runtime field names (for a hook that only has them as data).
- `scripts/concord-lint-for-lock.sh` (new, see "concord-lint"). `concord-lint` now checks `error_at!` / `error_at_anyhow!`, and a local registry can declare wrapper macros with `[lint] log_macros`.

0.4.0 also carries 0.3.1's `context::add_from_baggage`.

## 0.3.1

Additive: `context::add_from_baggage(header)`, the replacement for the
adoption `link_from` no longer does (see below).

## Breaking changes in 0.3.0

- `AttrValue::U64` is gone (nothing produced it; the derive records `u64` as strings).
- `carrier::link_from` no longer adopts the linked span's baggage. Callers that
  relied on it add the labels they need with `context::add`: multimatrix-ops
  `src/trace.rs:23` (`resend_span` relies on adoption), `:102`, `:136` and
  `fleet.rs:72`; multimatrix `src/main.rs:869`; grorg-server `events.rs:252`;
  gait-acp `trace_meta.rs:32`; gait `trace.rs:244` (the span is never entered,
  so use `span.in_scope(|| context::add(..))`).
- `http_server::IngressService`'s future is `ResponseFuture`, and the inner
  service must return `http::Response<_>`. Its request carries a `ServerSpan`.
- HTTP server span names change from the request path to the method (`GET`),
  or `GET /route` once the service calls `ServerSpan::record_route`.
- Baggage holds at most 24 entries (was 16); `tracestate` is capped at 512
  characters (was 256) and shortened per W3C instead of dropped.
- `concord-lint` has two new rules, `label-debug` and `reserved-label-prefix`.
- `testing::capture_exclusive()` also clears the exported spans.
- The `concord.` namespace is reserved for baggage keys (`concord.label.` is the
  stamping prefix, `concord.label_conflicts` is the conflict report).

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

For a record made on behalf of other code, `concord_telemetry::emit_at(file, line,
module, || ..)` runs a closure with that location, and
`error_at!(file, line, module, &err, source = .., fields..)` (and
`error_at_anyhow!`) is `error!` with it: an error hook in another crate passes the
failing code's `file!()`, `line!()` and `module_path!()`. Without `source =` the
fingerprint source is `module`. Field values are evaluated in a closure, so `.await`
and `?` do not work in them.

`concord_telemetry::emit_error_at(file, line, module, &err, &[(&'static str, AttrValue)])`
is the same record with field names known only at run time (an error hook's
`&[(name, value)]`, which `tracing::event!` cannot take). Each pair becomes a record
attribute of its own type (an `AttrValue::I64` stays an integer). The slice's names
are not checked by `concord-lint`; the wrapper's own call sites are, once the wrapper
is declared in `[lint] log_macros`.

### Version

Pass `concord_telemetry::version!()` as `Config`'s `service_version`. It expands
in your crate to the `CONCORD_GIT_SHA` build-time variable when set and
non-empty, else `CARGO_PKG_VERSION`, so a build that sets
`CONCORD_GIT_SHA=<commit>` reports its own commit. A run-time
`OTEL_RESOURCE_ATTRIBUTES=service.version=...` still wins.

### Build info

`Config::new(name, version!()).with_build_info(build_info!())` adds the
resource attributes `vcs.repository.name` (from `CONCORD_GIT_REPO`) and
`vcs.ref.head.revision` (from `CONCORD_GIT_SHA`), so telemetry can be joined to
a code graph on repository, commit, repo-relative `code.file.path` and
`code.line.number`. `repo!()` alone is the repository name. Both variables are
read at build time of each binary's own crate (`CONCORD_GIT_REPO=grorg
CONCORD_GIT_SHA=$(git rev-parse HEAD) cargo build`); unset or empty, the
attribute is omitted. `OTEL_RESOURCE_ATTRIBUTES` may override either at run
time; empty values there are ignored.

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
.extern_path(".concord_telemetry.v1", concord_telemetry_proto::RUST_EXTERN_PATH)
```

### Importing `principal.proto` from your build

An event proto with a `Principal` field imports
`concord_telemetry/v1/principal.proto`. Build scripts get its include path from
the `concord-telemetry-proto` crate (its own repo), which has no dependency on
the OpenTelemetry stack (never put `concord-telemetry` itself in
`[build-dependencies]`: that compiles it into your build-script graph). Generated
code names `::concord_telemetry_proto::v1::*`, so the crate is a normal dependency
too:

```toml
[dependencies]
concord-telemetry = { git = "https://github.com/qtpi-bonding/concord-telemetry.git", rev = "<revision>" }
concord-telemetry-proto = { git = "https://github.com/qtpi-bonding/concord-telemetry-proto.git", rev = "<revision>" }
[build-dependencies]
concord-telemetry-proto = { git = "https://github.com/qtpi-bonding/concord-telemetry-proto.git", rev = "<revision>" }
```

```rust
// build.rs
let includes = [PathBuf::from("proto"), concord_telemetry_proto::proto_include_dir()];
let descriptor = PathBuf::from(std::env::var("OUT_DIR")?).join("descriptor.bin");
prost_build::Config::new()   // or tonic_build::configure()
    .file_descriptor_set_path(&descriptor)
    .extern_path(".concord_telemetry.v1", concord_telemetry_proto::RUST_EXTERN_PATH)
    .compile_protos(&["proto/yours/v1/events.proto"], &includes)?;
pbjson_build::Builder::new()
    .register_descriptors(&std::fs::read(&descriptor)?)?
    .extern_path(".concord_telemetry.v1", concord_telemetry_proto::RUST_EXTERN_PATH)
    .build(&[".yours.v1"])?;
```

Both `extern_path` calls are needed: prost for the message, pbjson for serde.
The proto repo carries a working copy of this build (`proto-consumer-test/`).

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
text. `error.message` and each chain link's text are cut to fit 8192 bytes together
(`error.message` at most 4096, the serialized `error.chain` at most 4096), on a
character boundary, ending in `...[truncated]`; codes, classes and
fingerprints are never cut. A panic is recorded as `concord.process.panic` (class unknown, source the
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
`BAGGAGE` (`set_parent_from_env`); `tracestate` is not carried that way. A parent that spawns a child sets
`propagation::inject_env(&carrier)`'s pairs on the child's environment. A crate that
spawns processes but must stay free of OpenTelemetry takes the variable's
name from `concord_telemetry_proto::TRACEPARENT_ENV`. Without an
exporter there are no trace ids and nothing is sent. The `grpc.server` and
`grpc.client` spans are still created, and stay enabled whatever `RUST_LOG`
says, including a directive naming `concord_telemetry::grpc` itself. A server
joins any valid `traceparent` it receives, so an untrusted caller chooses the
trace id and the sampled flag of its own requests.

### Carriers and baggage

`concord_telemetry::carrier` moves a trace across a boundary as a
`TraceCarrier` (the three header values; `concord_telemetry_proto::v1::TraceCarrier`
is its protobuf form, `concord_telemetry_proto::v1::field` the conversion): `capture()` takes the current span's, `parent_on` joins it,
`link_from` starts a new trace linked to it, and `root(name)` starts one with
no parent. `extract_required(hop, ..)` is `parent_on` for a hop that must
carry a trace: with an exporter and no valid `traceparent` it records
`concord.trace.missing` with a `hop` field. `extract_ingress(name, ..)` is
silent on an empty carrier and sets the span attribute `ingress`; the gRPC
`ServerLayer` is an ingress.

`context::add("action_id", id)` puts a label in the current span's baggage:
children and outgoing carriers inherit it, and it is a span attribute. A later
`add` of the same key replaces the value. Baggage is labels only, never
authorization or routing. At most 24 entries (`context::MAX_ENTRIES`), keys up
to 64 bytes, values up to 256, and the header up to 8192 bytes of encoded text
(W3C Baggage's limit). `tracestate` stays at 512: over that, entries longer
than 128 characters go first, then whole entries from the end, and the header is
never dropped whole. Over a cap, in the reserved namespace, or malformed, an entry is dropped and the
header's drops are recorded once as `concord.trace.baggage_dropped` with
`dropped`; the request proceeds. Without an exporter every one of these is a
no-op, `concord.trace.missing` never fires, and local output is unchanged.

`context::add_from_baggage(header)` (0.3.1) adds every label of a `baggage`
header string to the current span, like `add` for each entry in header order
(latest wins, an unchanged value is not stamped again). The header is parsed
with the incoming-carrier rules, and what the parse or the caps drop is
recorded once as `baggage_dropped`. It replaces per-key helpers written
around `add`. Like `add` it needs the span entered:
`span.in_scope(|| context::add_from_baggage(h))`.

`link_from` carries span context only, as OpenTelemetry links do: it does not
change the span's labels. A caller that wants a linked span's labels calls
`context::add` for each, or hands the header to `context::add_from_baggage`
(0.3.1, above); on a span that is not entered, wrap either in
`span.in_scope(|| ..)`.
`parent_on` on a span that was already entered is refused: it cannot re-parent
and it leaves the labels as they are.

How labels become attributes. Inside the crate the baggage layer stamps
inherited labels, and `add` stamps its own, under `concord.label.<key>`; the
layer is private, so a custom provider never sees those keys itself. A span
processor, `LabelCollapse`, wrapped around the exporting one, resolves each
span when it ends: a repeated key (re-recorded fields, two `error!` calls)
exports its last value once, which the SDK does not do on its own; a
`concord.label.<k>` is renamed to `<k>`, unless the span has its own `<k>`, in
which case the span's value wins and, when the exported strings differ, the key
is listed in the span attribute `concord.label_conflicts` (keys only, never
values). `init` and `testing` install the wrapper. With your own tracer
provider, build it with
`SdkTracerProvider::builder().with_max_attributes_per_span(512).with_span_processor(LabelCollapse(your_processor))`
or repeated keys are exported as repeats and labels stay under their
`concord.label.` keys. The whole `concord.` namespace is reserved:
`context::add` refuses a key in it and an incoming baggage key in it is dropped
and counted.

Limits. A span created but never entered gets no inherited labels. Labels land
after the span starts, so samplers and `SpanProcessor::on_start` never see them
(tracing-opentelemetry keeps its span data private). Each changed label value is one stamped attribute until the span ends, so `init` raises the provider's `max_attributes_per_span` to 512; the collapse cannot recover attributes the SDK has already dropped. Logs differ: an event's own field wins
over a label of the same name, silently. tracing-subscriber's fmt layer formats
a user's `Debug` under its own lock, which a `Debug` impl that logs can
deadlock on; nothing here changes that, and nothing here runs user `Debug`.

## Testing

Enable the `testing` feature in dev-dependencies. `testing::capture()` installs
one process-global subscriber (panics if one is already set) and returns a
`Capture`; find events by a domain value you created:
`cap.single_event_where::<E>(|e| e.source_record_id == id)`. Tests never call
`init`. `capture_exclusive()` serializes and clears the store, spans included; other
tests using plain `capture()` in the same binary still write spans, so read
them with `spans_in_trace(&span)`. `span_attr(&span, key)` panics when a span
exports a key twice, and `assert_no_label_conflict(&spans)` fails on any
`concord.label_conflicts`. Spans reach the store through `LabelCollapse`, so a
test sees what the collector sees. If your tests already install a global subscriber, add
`testing::layer()` to it and read with `testing::reader()`. Both paths were
confirmed against grorg (tonic server tests) and structmux (daemon tests).

## concord-lint

Install at the rev you pin (or use `scripts/concord-lint-for-lock.sh`, below): `cargo install --git https://github.com/qtpi-bonding/concord-telemetry --rev <rev> concord-lint`.
`concord-lint [--local fields.local.toml] [--baseline lint-baseline.txt [--write-baseline]] <paths...>`
checks tracing event macros (messages are fixed strings, no interpolation or
arguments), field names against `fields.toml` plus your local overlay, never-log
names everywhere, and `process::exit` in binaries. Exit 0 clean, 1 violations,
2 usage or parse error. The baseline counts repeats, so a new copy of a
baselined violation is new. It parses source and never compiles your crate.
Label rules: a label (a field marked `label = true` in `fields.toml`, or in your
`fields.local.toml`) must not be recorded through Debug, by `?` in a span macro or
`#[instrument(fields(..))]`, or implicitly as an `#[instrument]` parameter whose
type tracing-attributes records through Debug and that is not skipped or shadowed by
`fields(..)` (`label-debug`); and no field is in the reserved `concord.` namespace
(`reserved-label-prefix`). Known limits: other span-macro and `#[instrument]` fields are not checked against the registry;
`process::exit` in modules of a binary (`mod cli;`) and a bare `exit(1)` after
`use std::process::exit` are not detected.

To lint at the rev a repo's `Cargo.lock` pins (the `concord-lint` package, git source):

```sh
rev=$(scripts/concord-lint-for-lock.sh path/to/Cargo.lock)   # prints the rev
~/.cache/concord-lint/$rev/bin/concord-lint --baseline lint-baseline.txt src
```

The script reads the `concord-lint` package's git source from the lock and, if
`~/.cache/concord-lint/<rev>/bin/concord-lint` is missing, runs `cargo install
--locked --root ~/.cache/concord-lint/<rev> --git <url> --rev <rev> concord-lint`
(through `$CARGO_SLOT`, else `cargo-slot` on PATH, else plain cargo; a nested
cargo-slot call runs straight through, so no deadlock). A lock with no
`concord-lint` package uses the `concord-telemetry` package's rev (same repo). It exits
non-zero if neither is git-sourced, or if either is locked at more than one rev. Consumers find it in the
checkout cargo made, as `check-otel-dependents.sh` is found, via `cargo metadata`.

`error_at!` and `error_at_anyhow!` are checked like `error!`; their first four
arguments (file, line, module, error) are skipped and the rest are fields.

A repo's own wrapper macro, such as `record_error!(err, name = value)`, is not
known to the scanner by default, so its fields are not checked. Declare it in the
local registry (the file passed as `--local`, e.g. `fields.local.toml`):

```toml
[lint]
log_macros = ["record_error"]
```

A declared macro (bare name, matched with or without a path such as
`crate::record_error!`) is treated like `error!`: the first argument is the error,
`source = ..` is skipped, every other `name = value` is checked against the
registry. Built-in error macros, tracing macros (`trace`, `debug`, `info`, `warn`, `event`, `span`) and non-identifiers are rejected. Undeclared
wrappers are ignored (pinned by tests). Run the lint with `--local` for any repo
that declares wrappers.

## concord-lint ratchet rules

The lint tool reports `ratchet.result_string`, `ratchet.dropped_result`,
`ratchet.error_to_string`, `ratchet.untraced_spawn`, and `ratchet.raw_channel`.
These findings use the same counting baseline mechanism as the existing rules;
test files and `#[cfg(test)]` modules are skipped.

## Checks

`scripts/check-otel-dependents.sh [-p PACKAGE] ALLOWED_CRATE...` is reusable in
your own workspace: it fails if any crate outside the OpenTelemetry family and
the crates you name (normally `concord-telemetry`) depends directly on
`opentelemetry`, using `cargo tree -e normal -i opentelemetry --depth 1`. Without
`-p` it checks the whole workspace. It runs cargo through `$CARGO_SLOT` when set.
Copy it or call it from a pinned checkout; do not rewrite it in your check script.

`scripts/check.sh` (through the compile slots): fmt, clippy, tests, the lock-script test, the
no-default-features build, the opentelemetry dependents allowlist (`scripts/check-otel-dependents.sh`), no tonic, no
rustls in the default tree, and the lint on this repo. `collector/otel-collector.yaml`
is an example collector config for the end-to-end stack.
