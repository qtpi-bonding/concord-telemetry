# concord-telemetry

One `init`, domain events and coded error records on OpenTelemetry. A repo's
binaries and its own internal crates (e.g. an `ops` crate that emits domain
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

## Features and the off build

| Feature | Default | Adds |
| --- | --- | --- |
| `otel` | yes | OpenTelemetry export: OTLP exporters, trace ids, span processors. Without it the whole API still compiles and runs, but nothing is exported and no trace ids exist; labels and causes work in every build. |
| `tls` | no | HTTPS to the collector (reqwest's rustls); implies `otel`. |
| `testing` | no | The `testing` module; implies `otel`. |
| `grpc` | no | `grpc::ServerLayer` / `grpc::client` over `http::Request`; no tonic. |
| `grpc-status` | no | `grpc::reject`, `reject_anyhow`, `reject_anyhow_redacted` (builds tonic `Status` values); implies `grpc`. |
| `http-server` | no | `http_server::IngressLayer`. |
| `channel` | no | `channel::traced`, `channel::spawn_traced`. |
| `lifecycle` | no | `lifecycle::spawn_alive` and `lifecycle::shutdown_signal` (tokio). |

`otel` is a default feature. To build without OpenTelemetry, a consumer sets
`default-features = false` on its `concord-telemetry` dependency at the
workspace root (`[workspace.dependencies]`) and adds `features = ["otel"]` on
each member that exports (binaries that ship telemetry). Members that inherit the
workspace entry unchanged get the off build. Features unify across a workspace:
if any crate in the build turns `otel` on, every crate sees it on, and the
API then behaves as off until `OTEL_EXPORTER_OTLP_ENDPOINT` is set (no
exporter, no thread, an inert `Guard`). Check the result with
`scripts/check-has-telemetry.sh` (must pass for an exporting member) and
`scripts/check-no-telemetry.sh` (must pass for a member that must stay free of
telemetry); both judge by the package names in `cargo tree -e normal`.

With `otel` off: `init` installs the local layer only (`Text`, `Json`,
`JsonFile`, `Off` all work) plus the label layer, `Guard` is empty,
`carrier::capture()` carries `baggage` (no `traceparent`), `parent_on` and
`extract_*` adopt baggage but set no trace, `link_from` is a no-op, the label
readers work, the `grpc` and `http_server` layers still open their spans, and the
`Event` derive and `error!` macros still write their fields to the local output.
The `concord_telemetry::__private` re-exports of `opentelemetry` exist only with `otel`.

## Init

```rust
let _guard = concord_telemetry::init(concord_telemetry::Config::new("my-service", env!("CARGO_PKG_VERSION")))?;
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

### Init failure

`concord_telemetry::report_init_failure(service, &err)` writes one JSON line
to stderr, for a binary whose `init` failed before any logger exists:

```rust
let _guard = match concord_telemetry::init(config) {
    Ok(guard) => guard,
    Err(err) => {
        concord_telemetry::report_init_failure("my-service", &err);
        return std::process::ExitCode::FAILURE;
    }
};
```

The line has exactly the keys `level` (`"error"`), `code`
(`"telemetry_init_failed"`), `service` and `message`, in that order. `message`
is the error's text followed by each `source()`, joined with `": "`.
`init_failure_line(service, &err)` returns the same line without the newline.
`Config`, `LocalOutput` and `InitError` are `#[non_exhaustive]`: build a
`Config` with `Config::new` and assign fields, and match `InitError` with a
`_` arm. `InitError::Exporter` holds a boxed error.

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

### Resource keys

`concord_telemetry::RESOURCE_KEYS` lists the keys the library sets or passes
through: `service.name`, `service.version`, `service.instance.id`, `host.name`,
`vcs.repository.name`, `vcs.ref.head.revision`, `deployment.environment.name`,
`agent.id`, `place.id` and `principal.*`. The last three non-principal keys
(`deployment.environment.name`, `agent.id`, `place.id`) are set only through
`OTEL_RESOURCE_ATTRIBUTES`; the old `deployment.environment` is passed through
as is, never translated. A key starting `concord.` in the environment is
skipped (reserved namespace).

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
is declared in `[lint] log_macros`. A pair wins over a label of the same name. A pair whose
name is on the shared never-log list is dropped, and a `concord.error.field_dropped`
record (`FIELD_DROPPED_CODE`, field `field`; the name, never the value) says so. The pairs go
onto exported records only: the local fmt/JSON lines print the tracing event's own fields, so they do not show them.

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
read at build time of each binary's own crate (`CONCORD_GIT_REPO=my-repo
CONCORD_GIT_SHA=$(git rev-parse HEAD) cargo build`); unset or empty, the
attribute is omitted. `OTEL_RESOURCE_ATTRIBUTES` may override either at run
time; empty values there are ignored.

## Trace-aware boundaries

The optional `channel` feature provides `channel::traced` and
`channel::spawn_traced`, which carry the current tracing span (including its
baggage) across Tokio channels and spawned tasks. `TracedSender` also has `try_send`, `reserve` (a `TracedPermit` whose `send` records the span current at that call), `closed` and `is_closed`; `TracedReceiver` has `poll_recv` and `close` (0.4.1). The optional `http-server`
feature provides `http_server::IngressLayer` for joining W3C HTTP ingress
headers in an `http.server` span.

## Lifecycle

`concord_telemetry::lifecycle` emits the service lifecycle events, so every
service reports the same ones. The service name, version and instance id come from
`init` (the Resource's `service.name`, `service.version`, `service.instance.id`),
never from a caller. `init` must have succeeded first: before it, each function
returns `Err(LifecycleError::NotInitialised)` (code `concord.lifecycle.not_initialised`).

| Event type name | Message fields |
| --- | --- |
| `concord_telemetry.v1.ServiceStarted` | `source_record_id`, `service`, `version` |
| `concord_telemetry.v1.ServiceStopping` | `source_record_id`, `service`, `reason` (optional), `uptime_ms` |
| `concord_telemetry.v1.ServiceAlive` | `source_record_id`, `service`, `uptime_ms` |

`source_record_id` is the instance id (the 16-digit hex `service.instance.id`),
not the service name. `uptime_ms` counts from `init`. Functions: `started()`,
`stopping(reason: Option<&str>)`, `spawn_alive(period)` (feature `lifecycle`; a
heartbeat until the returned `Alive` is dropped, first tick after one period) and
`shutdown_signal().await` (feature `lifecycle`; waits for ctrl-c or SIGTERM,
emits `stopping(Some("ctrl_c"))` or `stopping(Some("sigterm"))` once and returns).
`started` and `stopping` are `#[track_caller]`: the record's `code.*` location is the call site.

A smoke check ("golden loop") that a service came up looks for a
`concord_telemetry.v1.ServiceStarted` record whose `service` is the expected
service, whose `version` is the expected build, and whose `source_record_id`
is non-empty (an instance id, one per process start); a `ServiceAlive` stream
with the same `source_record_id` shows it is still running.

## gRPC status

With `grpc-status` (adds tonic, no TLS): `grpc::reject(method, err)` (a
`CodedError`), `grpc::reject_anyhow(method, &err)` and
`grpc::reject_anyhow_redacted(method, &err, fixed_message)` record the error with
the `rpc.method` field at the caller's code location and return a `tonic::Status`
whose code follows the error class (`reject_anyhow_redacted` puts
`fixed_message` in the status and keeps the chain in the record only). The crate's
`grpc` feature alone stays tonic-free.

## Labels

Labels are the baggage entries that travel with a trace (the fields marked
`label = true` in `fields.toml`: `action_id`, `agent_id`, `daemon_id`,
`delivery_id`, `instance_id`, `message_id`, `place_id`, `request_id`,
`session_id`, `source_instance_id`, `source_turn_id`, `turn_id`; `concord_fields::label_names()` lists them).
Helpers, all public:

- `labels::parse(header)` and `labels::format(entries)`: pure, percent-decoding,
  caps applied (24 entries, keys 64 bytes, values 256 bytes, header 8192 bytes),
  properties dropped, invalid entries skipped. They work with `otel` off.
- `labels::of(span, key)`, `labels::add_once(span, key, value)` (no-op when the
  label is present or the value empty) and `labels::record_once(span, field, value)`
  (a `span.record` unless the label exists).
- `carrier::with_label(&carrier, key, value)` (replace or append; an invalid
  label leaves the carrier as it was and records `concord.trace.baggage_dropped`),
  `carrier::without_label(&carrier, key)`, and
  `carrier::parent_on_or_env(span, Option<&carrier>)` (a carrier with a
  `traceparent` wins, else `TRACEPARENT` / `BAGGAGE` from the environment).
- `concord_telemetry::clip(value)` cuts a value to `concord_fields::FREE_TEXT_MAX`
  (1024) bytes on a character boundary, for a free-text field.

### Session ids

Three names, three meanings: `session_id` (a label: the Session the work belongs
to), `target_session_id` (the Session an operation is about, when that differs
from the `session_id` label) and `acp_session_id` (the id held with an ACP agent,
apart from the upstream session id). A name that belongs to one application
stays in that application's `fields.local.toml`.

## Causes

Causes are the registered ids a request was caused by (an action, a turn, a
message, an instance), from the `[causes]` table of `fields.toml`. They are
not labels: they never travel as baggage and never stamp `concord.label.*`.
Each cause is a `Cause { kind, id }` (the `concord_telemetry.v1` proto type,
re-exported here).

- Capture at the door. Every entry point that adopts a carrier (gRPC and HTTP
  ingress, `parent_on`, `set_parent_from_env`, NATS `parent_on`) stores a frozen
  snapshot of the causes its baggage named, in the span. An entry with no
  baggage stores an empty snapshot, so it is a boundary.
- `causes::capture_incoming()` returns the nearest entry's snapshot. Later
  `context::add` calls never change it: a handler that writes its own
  `action_id` still reports the caller's cause.
- `causes::scope(causes, fut)` and `causes::sync_scope(causes, f)` hold the
  snapshot as a task-local, so `causes::incoming()` reads it across awaits. The
  gRPC and HTTP layers scope their handlers themselves. A spawned task does not
  inherit it: pass the causes to `scope` explicitly. `scope_with(extra, fut)`
  extends the incoming causes for one inner call only.
- `causes::excluding(&[(kind, id)])` removes exactly the pairs given. Which
  records contain which causes is the caller's table, not this crate's.
- `Causes::new`, `with` and `from_entries` drop a kind that is not in
  `[causes]`, an empty id, an id over 256 bytes, a repeat, and anything past 8
  causes. One `concord.trace.causes_dropped` WARN (field `dropped`) per
  construction that dropped anything.
- `Causes::to_json()` is the `causes` span attribute, written with the `otel`
  feature only (`[{"kind":"action","id":"A1"}]`). Without `otel` it is not
  recorded anywhere; local JSON lines do not carry it.

## Events

```rust
concord_telemetry::event!(ActionStatusChanged { source_record_id: id.clone(), /* ... */ });
```

Generate event messages with prost and add the derive from `build.rs`, derive
line first and helper second, with `message_attribute` (never `type_attribute`,
which also reaches enums and oneofs):

```rust
.message_attribute(".app.events", "#[derive(::concord_telemetry::Event)]")
.message_attribute(".app.events", "#[concord_event(package = \"app.events.v1\")]")
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

`concord_telemetry::warn!(&err, source = X::NAME, …)` and `warn_anyhow!` make the same record at WARN
(`level` is `"WARN"`) for a degraded but handled condition. The span is not marked failed: no
`error.code` attribute and no error status.

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
`testing::layer()` to it and read with `testing::reader()`. Both paths are
covered by tests (a tonic server and a daemon).

### Testing helpers

`testing::check_attributes(&registry, span_name, &pairs)` panics on a repeated
key (naming the span, the key and both values), on a key that is neither
registered nor written by the tracing layer (`target`, `level`, `busy_ns`,
`idle_ns` and the prefixes `otel.`, `code.`, `thread.`), and on a `free_text`
field over `FREE_TEXT_MAX`. `testing::spans_checked(&registry, prefix)` returns
the exported spans whose name starts with `prefix`, after running that check
on each. Build the registry with `concord_fields::Registry::shared()` plus
`.with_local(src)` for a local overlay; `testing::{Registry, label_names}` are
re-exported. `testing::trace_env(traceparent, baggage)` sets (or, for `None`,
removes) `TRACEPARENT` and `BAGGAGE`, holds a process-wide lock and restores
the previous values when the guard drops, including on panic;
`testing::with_trace_env(tp, bg, || ..)` is the closure form.

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

## Checks and scripts

- `scripts/check.sh` is the whole gate, every cargo call through the compile
  slot (`$CARGO_SLOT`, else `cargo-slot` on PATH, else cargo): fmt; clippy with
  `-D warnings` for the workspace with all features and for the off build; tests
  for the workspace, the off build, `grpc,channel,http-server` and
  `lifecycle,grpc-status`; the off-build check; the telemetry-in-the-tree gates;
  the opentelemetry dependents allowlist; no tonic with `grpc`, tonic with
  `grpc-status`, no rustls in the default tree; the script tests; and the
  self-lint over `concord-telemetry/src concord-lint/src concord-fields/src`
  with `--baseline lint-baseline.txt` (the baseline may only shrink).
- `scripts/check-has-telemetry.sh [-p PKG]...` fails unless the exact package
  `opentelemetry` is in each package's normal tree.
- `scripts/check-no-telemetry.sh [-p PKG]... [--features F] [--also NAME]...`
  fails when `concord-telemetry`, `concord-telemetry-derive`,
  `tracing-opentelemetry`, any `opentelemetry*` package or an `--also` name is in
  the normal tree. Both decide by the package names in `cargo tree -e normal`,
  not by exit status: current cargo exits 0 and prints "nothing to print" for
  `cargo tree -i <absent package>`.
- `scripts/check-otel-dependents.sh [-C DIR] [-p PACKAGE] ALLOWED_CRATE...` fails
  if any crate outside the OpenTelemetry family and the crates you name
  (normally `concord-telemetry`) depends directly on `opentelemetry`; without
  `-p` it checks the whole workspace.
- `scripts/concord-lint-for-lock.sh [--path] Cargo.lock` prints the pinned rev,
  or with `--path` the installed binary's path.

Consumers call these from the checkout cargo made for their pin; find it with:

```sh
cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="concord-telemetry") | .manifest_path | sub("/concord-telemetry/Cargo.toml$"; "")'
```

and run `<that dir>/scripts/check-has-telemetry.sh -p <member>` and so on.
Do not rewrite them in your own check script. `collector/otel-collector.yaml`
is an example collector config for the end-to-end stack.
