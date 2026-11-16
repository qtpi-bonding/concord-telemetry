# Changelog

## 0.5.0

concord-telemetry 0.5.0, concord-fields 0.2.0, concord-telemetry-derive 0.2.0,
concord-lint 0.2.0. Pin all four at the same revision.

### Added

- Feature `otel` (default). Without it the API is the same and nothing is
  exported; see "Features and the off build" in the README.
- `report_init_failure(service, &err)` and `init_failure_line(service, &err)`
  (every build). Published keys, in this order:
  `{"level":"error","code":"telemetry_init_failed","service":"<service>","message":"<error: source: source>"}`.
  They replace a per-application init-failure line. Old per-application key
  names a log search or monitor may still match are `error`, `detail` and
  `event`: `error` and `detail` (the error text) are now `message`, `event`
  (the failure's name) is now `code`, whose value is the fixed
  `telemetry_init_failed`, and `service` and `level` are new keys.
  Search logs and monitors for the old keys and move them to the new ones.
- `lifecycle` module (events registered in `fields.toml`):
  `concord_telemetry.v1.ServiceStarted` (`source_record_id`, `service`,
  `version`), `concord_telemetry.v1.ServiceStopping` (`source_record_id`,
  `service`, `reason`, `uptime_ms`) and `concord_telemetry.v1.ServiceAlive`
  (`source_record_id`, `service`, `uptime_ms`); functions `started`, `stopping`,
  `spawn_alive`, `shutdown_signal` (`shutdown_signal` and `spawn_alive` need the
  `lifecycle` feature; `shutdown_signal` emits `ServiceStopping` with reason
  `ctrl_c` or `sigterm`). `source_record_id` is the instance id (the Resource's
  `service.instance.id`); `service` and `version` come from `init`.
- Feature `grpc-status`: `grpc::reject`, `grpc::reject_anyhow`,
  `grpc::reject_anyhow_redacted`.
- Public label helpers: `labels::{parse, format, of, add_once, record_once}`,
  `carrier::{with_label, without_label, parent_on_or_env}`, `clip`.
- Testing helpers: `testing::{check_attributes, spans_checked, trace_env,
  with_trace_env, label_names, Registry}`.
- `RESOURCE_KEYS`: `service.name`, `service.version`, `service.instance.id`,
  `host.name`, `vcs.repository.name`, `vcs.ref.head.revision`,
  `deployment.environment.name`, `agent.id`, `place.id`, `principal.kind`,
  `principal.id`, `principal.verified_by`, `principal.via`. `deployment.environment.name`,
  `agent.id` and `place.id` are set through `OTEL_RESOURCE_ATTRIBUTES` only. A key
  starting `concord.` there is skipped; the old `deployment.environment` is
  passed through as written, never translated.
- `fields.toml`: every entry has a `type` (`str`, `int`, `float`, `bool`); fields
  that carry free text are marked `free_text = true` (cap 1024 bytes); the
  session-id family is `session_id`, `target_session_id`, `acp_session_id`;
  `error.code`, `error.class`, `error.fingerprint`, `error.source`, `ingress`,
  `service`, `version` and `uptime_ms` are registered; many fields that several
  applications each defined locally are now shared (see below).
- `scripts/check-has-telemetry.sh`, `scripts/check-no-telemetry.sh`,
  `scripts/check-otel-dependents.sh -C DIR`, `scripts/concord-lint-for-lock.sh --path`;
  `scripts/check.sh` is the full gate.
- The `Event` derive checks a field's kind against its registered type.

### Breaking changes

- `Config`, `LocalOutput` and `InitError` are `#[non_exhaustive]` (no struct
  literals, `_` arms) and derive `Clone`, `Debug`, `PartialEq` (`LocalOutput` also `Eq`).
  `InitError::Exporter` holds a `Box<dyn Error + Send + Sync>`.
- Feature `reqwest-blocking-client` is removed; use `otel`.
- `concord_lint::registry::{Registry, FieldVerdict, FieldDef}` moved to
  `concord_fields::{Registry, FieldVerdict, FieldDef}`; `concord_fields::parse`
  returns a typed `ParseError`.
- `fields.toml` and `fields.local.toml` entries must have a `type`; a
  `label` field must be `str` and cannot be `free_text`. The lint refuses a local
  name that shadows a shared one, so a promoted name must be deleted locally.
- `concord-lint` exits 2 on a local registry without `type`s; its checks and
  baseline format are otherwise unchanged. The self-lint baseline is empty.
- `testing::stamp_count`, `set_enter_hook` and `set_otel_active` are hidden.
- The lifecycle events' `event.type` is `concord_telemetry.v1.ServiceStarted`
  (and `ServiceStopping`, `ServiceAlive`); a message defined by a consumer under
  its own package is not the same event.
- A baggage value is capped at 256 bytes (a consumer comment of 128 is wrong).
- `skipped` is a bool in the shared registry; the integer is `skipped_count`.

### Consumer impact

Every consumer:

- Re-pin `concord-telemetry` 0.5.0 and `concord-fields`, `-derive`, `concord-lint`
  at the same revision; run `scripts/concord-lint-for-lock.sh --path` for the lint.
- Delete every `fields.local.toml` entry that is now shared; every remaining
  local entry needs a `type`.
- Replace the local init-failure line with `report_init_failure`.
- Replace `Config { .. }` literals with `Config::new(..)` plus field assignment;
  add a `_` arm to matches on `LocalOutput` and `InitError`.
- Use `otel` instead of `reqwest-blocking-client`.
- Build off with `default-features = false` on the workspace dependency and
  `features = ["otel"]` on each exporting member; verify with
  `check-has-telemetry.sh` (and `check-no-telemetry.sh` for members that must stay free of it).

Consumer impact by kind of consumer (neutral headings; the library names no
application, so see the pins of your own repo):

- A consumer with its own service-started/stopping events and shutdown-signal
  code: replace them with `lifecycle::{started, stopping, shutdown_signal}`; the
  event type changes to `concord_telemetry.v1.ServiceStarted` / `ServiceStopping`,
  the `service` and `version` fields are set by the library, and
  `source_record_id` becomes the instance id (it was the service name or a
  machine id); delete local `service`, `version`, `reason`, `uptime_ms` and the
  `ServiceStarted` / `ServiceStopping` / `ServiceAlive` proto messages. Monitors,
  smoke checks (the golden loop) and the required-services list follow the new
  type name and key.
- A consumer that returns `tonic::Status` from a handler: adopt `grpc::reject*`
  (feature `grpc-status`). A consumer that must not link tonic keeps `grpc` only.
- A consumer with its own baggage helpers: use `carrier::with_label`,
  `carrier::without_label`, `carrier::parent_on_or_env` and
  `labels::add_once` / `labels::record_once` in place of local copies.
- A consumer whose tests list baggage keys or check span attributes by hand:
  use `testing::label_names()`, `testing::check_attributes`,
  `testing::spans_checked(&Registry::shared().with_local(..), prefix)` and
  `testing::with_trace_env` (env set/remove in tests).
- A consumer with an off shim of its own: delete it; the same API builds with
  `otel` off.
- A consumer with free-text fields (command lines, frames, JSON bodies, tails of
  output): mark them `free_text = true` locally and clip values with
  `concord_telemetry::clip` (cap 1024) or the checks fail.
- Renames that depend on the consumer's own field names:
  a field named `<app>_session_id` for another process's Session becomes
  `target_session_id`; a `principal_id` field becomes `principal`; an integer
  `skipped` becomes `skipped_count` (the shared `skipped` is bool); a field named
  `session` that holds a terminal-multiplexer session is renamed locally (never to
  `target_session_id`); an ACP session id local to one app becomes the shared
  `acp_session_id`; the local `<app>_session_id` stays local.
- A consumer whose own library re-exports mirrors of `Guard` or `LocalOutput`:
  remove them.
- Resource attributes: `deployment.environment` becomes
  `deployment.environment.name`; an application-prefixed agent-id attribute
  becomes `agent.id`; `place.id` is new; set them in the deployment's
  `OTEL_RESOURCE_ATTRIBUTES`. A script that writes events by hand aligns its
  resource keys with `RESOURCE_KEYS`.
- Deployment and monitoring repositories: update the pins, the
  `check-otel-dependents` call sites, monitor rules and smoke checks (the new
  event type, `source_record_id`, `service`, `version`) in the same change as the
  consumers' re-pin, and grep logs and monitors for the old init-failure keys
  (`error`, `detail`, `event`).
- A consumer that does not use `concord-telemetry` (or depends on it only for
  proto types): nothing changes.

Shared names promoted from local registries (delete the local copy, keep the
name; `command` is `free_text`): `outcome`, `ok`, `count`, `reason`, `kind`,
`command`, `listen`, `attempt`, `attempts`, `cause` ("What triggered an
operation or re-derivation"), `status`, `node`, `nodes`, `principal`, `file`,
`path`, `target`, `target_machine_id`, `skipped`, `skipped_count`, `cwd`, `pid`,
`key_id`, `message_count`, `error_code`, `target_session_id`, `acp_session_id`.
`scope`, `mode`, `phase` and `source` stay local on purpose (meanings diverge).

## 0.4.0 (breaking)

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

## 0.3.0 (breaking)

- `AttrValue::U64` is gone (nothing produced it; the derive records `u64` as strings).
- `carrier::link_from` no longer adopts the linked span's baggage. Callers that
  relied on it add the labels they need with `context::add` (on a span that is
  never entered, use `span.in_scope(|| context::add(..))`).
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
