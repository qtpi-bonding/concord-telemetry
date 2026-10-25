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

/// The text of `principal.proto`.
pub const PRINCIPAL_PROTO: &str = include_str!("../proto/concord_telemetry/v1/principal.proto");

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
}
