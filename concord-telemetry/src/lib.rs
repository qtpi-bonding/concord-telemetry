mod deadline;
mod init;
mod panic;
mod principal;

pub mod proto {
    pub use crate::principal::v1;
}

pub use init::{init, Config, Guard, InitError, LocalOutput, EVENT_TARGET};
pub use panic::PANIC_CODE;
pub use principal::{Principal, PrincipalKind, Verification};
