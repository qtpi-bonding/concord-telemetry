mod deadline;
mod error;
mod event;
mod init;
mod panic;
mod principal;

#[cfg(feature = "testing")]
pub mod testing;

pub mod proto {
    pub use crate::principal::v1;
}

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
