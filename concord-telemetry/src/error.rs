/// Emit a coded error record without recording its Display representation.
#[macro_export]
macro_rules! error {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let facts = $crate::__private::coded_error::Facts::of($err);
        $crate::__emit_error!(facts, $source $(, $($fields)*)?)
    }};
    ($err:expr $(, $($fields:tt)*)?) => {{
        let facts = $crate::__private::coded_error::Facts::of($err);
        $crate::__emit_error!(facts, ::core::module_path!() $(, $($fields)*)?)
    }};
}

/// Emit a record for an anyhow error, using its nearest attached coded facts.
#[macro_export]
macro_rules! error_anyhow {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let facts = $crate::__private::coded_error::facts_anyhow($err);
        $crate::__emit_error!(facts, $source $(, $($fields)*)?)
    }};
    // No `source = ...`: the error's own code is the source label, so the fingerprint is
    // stable per code (unlike `error!`, whose default is the calling module path).
    ($err:expr $(, $($fields:tt)*)?) => {{
        let facts = $crate::__private::coded_error::facts_anyhow($err);
        $crate::__emit_error!(facts, facts.code $(, $($fields)*)?)
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __emit_error {
    ($facts:expr, $source:expr $(, $($fields:tt)*)?) => {{
        let facts: $crate::__private::coded_error::Facts = $facts;
        let source: &str = $source;
        let fingerprint = $crate::__private::coded_error::fingerprint(
            facts.code,
            facts.class,
            source,
        );
        $crate::__private::tracing::event!(
            name: "error",
            target: $crate::EVENT_TARGET,
            $crate::__private::tracing::Level::ERROR,
            error.code = facts.code,
            error.class = facts.class.as_str(),
            error.fingerprint = fingerprint.as_str(),
            error.source = source,
            $($($fields)*)?
        );
    }};
}
