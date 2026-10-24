mod principal;

pub mod proto {
    pub use crate::principal::v1;
}

pub use principal::{Principal, PrincipalKind, Verification};