use std::collections::BTreeMap;

/// A value captured from a telemetry event.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Str(String),
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
}

pub type Attrs = BTreeMap<String, AttrValue>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RebuildError {
    #[error("missing event attribute: {0}")]
    Missing(String),
    #[error("wrong type for event attribute: {0}")]
    Type(String),
    #[error("could not parse event attribute: {0}")]
    Parse(String),
}

pub trait Event: Sized {
    const TYPE_NAME: &'static str;
    fn emit(&self);
    fn from_attributes(attrs: &Attrs) -> Result<Self, RebuildError>;
}

#[macro_export]
macro_rules! event {
    ($ev:expr) => {{
        $crate::Event::emit(&$ev);
    }};
}
