#[cfg(feature = "otel")]
mod baggage;
pub mod carrier;
mod coded;
#[cfg(feature = "otel")]
mod deadline;
pub mod error;
mod event;
mod init;
mod init_failure;
#[cfg(feature = "otel")]
mod label_collapse;
mod location;
mod logfmt;
mod panic;
pub mod propagation;

#[cfg(feature = "channel")]
pub mod channel;
#[cfg(feature = "http-server")]
pub mod http_server;

pub use carrier::context;

#[cfg(feature = "grpc")]
pub mod grpc;
#[cfg(feature = "testing")]
pub mod testing;

pub use carrier::{
    BAGGAGE_DROPPED_CODE, CARRIER_DROPPED_CODE, FIELD_DROPPED_CODE, TRACE_MISSING_CODE,
};
pub use concord_telemetry_proto::TraceCarrier;

pub use concord_telemetry_derive::Event;
pub use concord_telemetry_proto::v1::{Principal, PrincipalKind, Verification};
pub use error::{emit_error_at, error_text, set_error_text, ErrorText};
pub use event::{AttrValue, Attrs, Event, RebuildError};
pub use init::{
    init, BuildInfo, Config, Guard, InitError, LocalOutput, EVENT_TARGET, RESOURCE_KEYS,
};
pub use init_failure::{init_failure_line, report_init_failure};
#[cfg(feature = "otel")]
pub use label_collapse::LabelCollapse;
pub use location::emit_at;
pub use panic::PANIC_CODE;

#[doc(hidden)]
pub mod __private {
    pub use crate::error;
    pub use crate::error::record_error_span;
    pub use coded_error;
    #[cfg(feature = "otel")]
    pub use opentelemetry;
    pub use prost_types;
    pub use serde_json;
    pub use tracing;
    #[cfg(feature = "otel")]
    pub use tracing_opentelemetry;
}
