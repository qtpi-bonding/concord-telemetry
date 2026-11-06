//! The dependency-free pieces of concord-telemetry. [`TRACEPARENT_ENV`] names
//! the variable that carries trace context into a spawned process, for crates
//! that must stay free of OpenTelemetry.
//!
//! It also says where `concord_telemetry/v1/principal.proto` lives, for
//! consumers' build scripts. Add this crate under `[build-dependencies]` (it has no
//! dependencies of its own) and pass [`proto_include_dir`] as a protoc
//! include path. The generated Rust types are not here: point
//! `extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")`
//! at the runtime crate, in prost and pbjson alike.

use std::path::PathBuf;

/// The include root: contains `concord_telemetry/v1/principal.proto`, so a
/// proto imports it as `import "concord_telemetry/v1/principal.proto";`.
pub fn proto_include_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/proto"))
}

/// The proto package the extern paths must name.
pub const PROTO_PACKAGE_PATH: &str = ".concord_telemetry.v1";

/// The Rust module that holds the hand-written types for that package.
pub const RUST_EXTERN_PATH: &str = "::concord_telemetry::proto::v1";

/// The environment variable that carries a W3C `traceparent` into a hook or
/// CLI. `concord_telemetry::propagation` re-exports it.
pub const TRACEPARENT_ENV: &str = "TRACEPARENT";

/// The environment variable that carries W3C `baggage` into a hook or CLI.
pub const BAGGAGE_ENV: &str = "BAGGAGE";

/// The text of `principal.proto`.
pub const PRINCIPAL_PROTO: &str = include_str!("../proto/concord_telemetry/v1/principal.proto");

/// The text of `trace_carrier.proto`.
pub const TRACE_CARRIER_PROTO: &str =
    include_str!("../proto/concord_telemetry/v1/trace_carrier.proto");

/// W3C trace context and baggage as the three header values, for crates that
/// must stay free of OpenTelemetry. Each field is raw header text, unparsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceCarrier {
    pub traceparent: Option<String>,
    pub tracestate: Option<String>,
    pub baggage: Option<String>,
}

impl TraceCarrier {
    /// True when no header is set.
    pub fn is_empty(&self) -> bool {
        self.traceparent.is_none() && self.tracestate.is_none() && self.baggage.is_none()
    }

    /// The set headers as `(name, value)` pairs, in the order traceparent,
    /// tracestate, baggage. Unset headers are omitted.
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        [
            ("traceparent", &self.traceparent),
            ("tracestate", &self.tracestate),
            ("baggage", &self.baggage),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.clone().map(|v| (name, v)))
        .collect()
    }

    /// Reads each header through `get`, which returns the value for a header
    /// name or `None` when it is absent.
    pub fn from_pairs(get: impl Fn(&str) -> Option<String>) -> TraceCarrier {
        TraceCarrier {
            traceparent: get("traceparent"),
            tracestate: get("tracestate"),
            baggage: get("baggage"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_dir_holds_the_import_path() {
        assert!(proto_include_dir()
            .join("concord_telemetry/v1/principal.proto")
            .is_file());
        assert!(PRINCIPAL_PROTO.contains("package concord_telemetry.v1;"));
    }

    #[test]
    fn trace_carrier_pairs_round_trip() {
        let carrier = TraceCarrier {
            traceparent: Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".into()),
            tracestate: Some("vendor=abc".into()),
            baggage: Some("concord.principal=alice".into()),
        };
        let pairs = carrier.to_pairs();
        assert_eq!(
            pairs.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec!["traceparent", "tracestate", "baggage"]
        );
        let rebuilt = TraceCarrier::from_pairs(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        });
        assert_eq!(rebuilt, carrier);
    }

    #[test]
    fn empty_carrier_has_no_pairs() {
        assert!(TraceCarrier::default().to_pairs().is_empty());
        assert!(TraceCarrier::default().is_empty());
    }

    #[test]
    fn include_dir_holds_trace_carrier_proto() {
        assert!(proto_include_dir()
            .join("concord_telemetry/v1/trace_carrier.proto")
            .is_file());
        assert!(TRACE_CARRIER_PROTO.contains("message TraceCarrier"));
    }
}
