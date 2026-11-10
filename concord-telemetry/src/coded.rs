//! The one emitter of the library's own coded records.

/// Emits a coded error record: `error.code`, `error.class`, the fingerprint of
/// (code, class, source), `error.source`, then the caller's extra fields
/// (their names and types differ: `hop`, `dropped`, `header`).
macro_rules! emit_coded {
    ($level:expr, $code:expr, $class:expr, $source:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let class: ::coded_error::ErrorClass = $class;
        let source: &str = $source;
        let digest = ::coded_error::fingerprint($code, class, source);
        ::tracing::event!(
            name: "error",
            target: $crate::EVENT_TARGET,
            $level,
            error.code = $code,
            error.class = class.as_str(),
            error.fingerprint = digest.as_str(),
            error.source = source
            $(, $name = $value)*
        );
    }};
}

pub(crate) use emit_coded;
