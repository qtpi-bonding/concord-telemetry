mod baggage;
pub mod carrier;
mod deadline;
pub mod error;
mod event;
mod init;
mod location;
mod logfmt;
mod panic;
mod principal;
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

pub mod proto {
    pub mod v1 {
        pub use crate::carrier::v1::TraceCarrier;
        pub use crate::principal::v1::*;
    }
}

pub use carrier::{BAGGAGE_DROPPED_CODE, CARRIER_DROPPED_CODE, TRACE_MISSING_CODE};
pub use concord_telemetry_proto::TraceCarrier;

pub use concord_telemetry_derive::Event;
pub use error::{error_text, set_error_text, ErrorText};
pub use event::{AttrValue, Attrs, Event, RebuildError};
pub use init::{init, BuildInfo, Config, Guard, InitError, LocalOutput, EVENT_TARGET};
pub use panic::PANIC_CODE;
pub use principal::{Principal, PrincipalKind, Verification};

#[doc(hidden)]
pub mod __private {
    pub use crate::error;
    pub use crate::location::emit_at;
    pub use coded_error;
    pub use opentelemetry;
    pub use prost_types;
    pub use serde_json;
    pub use tracing;
    pub use tracing_opentelemetry;
}
