mod baggage;
pub mod carrier;
mod deadline;
mod error;
mod event;
mod init;
mod panic;
mod principal;
pub mod propagation;

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

pub use carrier::{BAGGAGE_DROPPED_CODE, TRACE_MISSING_CODE};
pub use concord_telemetry_proto::TraceCarrier;

pub use concord_telemetry_derive::Event;
pub use event::{AttrValue, Attrs, Event, RebuildError};
pub use init::{init, Config, Guard, InitError, LocalOutput, EVENT_TARGET};
pub use panic::PANIC_CODE;
pub use principal::{Principal, PrincipalKind, Verification};

#[doc(hidden)]
pub mod __private {
    pub use coded_error;
    pub use prost_types;
    pub use serde_json;
    pub use tracing;
}
