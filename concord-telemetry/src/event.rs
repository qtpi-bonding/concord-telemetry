use std::collections::BTreeMap;

/// A value captured from a telemetry event.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Str(String),
    I64(i64),
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

/// Emits a domain event, recording the macro's call site as the record's code
/// location. The argument is evaluated first, outside any closure, so `.await`
/// and `?` work inside it, and only borrowed, so the event stays usable.
#[macro_export]
macro_rules! event {
    ($ev:expr) => {{
        let event = &$ev;
        $crate::emit_at(
            ::core::file!(),
            ::core::line!(),
            ::core::module_path!(),
            || $crate::Event::emit(event),
        );
    }};
}

/// The version string to pass as `Config`'s `service_version`: the caller's
/// `CONCORD_GIT_SHA` build-time environment variable when set and non-empty,
/// else the caller's `CARGO_PKG_VERSION`. Both are read when the *caller's*
/// crate compiles, so each binary reports its own repo's commit.
///
/// ```
/// let config = concord_telemetry::Config::new("svc", concord_telemetry::version!());
/// # let _ = config;
/// ```
///
/// A deployment's `OTEL_RESOURCE_ATTRIBUTES=service.version=...` still wins at
/// run time.
#[macro_export]
macro_rules! version {
    () => {
        $crate::version!(@pick ::core::option_env!("CONCORD_GIT_SHA"), ::core::env!("CARGO_PKG_VERSION"))
    };
    (@pick $sha:expr, $pkg:expr) => {
        match $sha {
            ::core::option::Option::Some(sha) if !sha.is_empty() => sha,
            _ => $pkg,
        }
    };
}

/// The caller's `CONCORD_GIT_REPO` build-time environment variable (the repo
/// the binary is built from, e.g. `grorg`), else the empty string. Read when
/// the *caller's* crate compiles.
#[macro_export]
macro_rules! repo {
    () => {
        $crate::repo!(@pick ::core::option_env!("CONCORD_GIT_REPO"))
    };
    (@pick $repo:expr) => {
        match $repo {
            ::core::option::Option::Some(repo) => repo,
            ::core::option::Option::None => "",
        }
    };
}

/// A [`BuildInfo`](crate::BuildInfo) from the caller's build environment:
/// `repo!()` and the caller's `CONCORD_GIT_SHA` (empty when unset). Pass it
/// to `Config::with_build_info` so every record's resource says which
/// repository and commit the binary came from (`vcs.repository.name`,
/// `vcs.ref.head.revision`).
///
/// ```
/// let config = concord_telemetry::Config::new("svc", concord_telemetry::version!())
///     .with_build_info(concord_telemetry::build_info!());
/// # let _ = config;
/// ```
#[macro_export]
macro_rules! build_info {
    () => {
        $crate::BuildInfo {
            repository: $crate::repo!(),
            revision: match ::core::option_env!("CONCORD_GIT_SHA") {
                ::core::option::Option::Some(sha) => sha,
                ::core::option::Option::None => "",
            },
        }
    };
}
