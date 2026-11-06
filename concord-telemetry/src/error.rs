use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Once;

/// Which error messages are included in error records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorText {
    Own,
    Full,
}

static ERROR_TEXT: AtomicU8 = AtomicU8::new(1);
static UNKNOWN_TEXT_WARNING: Once = Once::new();

pub fn set_error_text(t: ErrorText) {
    ERROR_TEXT.store(
        match t {
            ErrorText::Own => 0,
            ErrorText::Full => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn error_text() -> ErrorText {
    match ERROR_TEXT.load(Ordering::Relaxed) {
        0 => ErrorText::Own,
        1 => ErrorText::Full,
        _ => {
            UNKNOWN_TEXT_WARNING
                .call_once(|| tracing::warn!("unknown error text mode; using full"));
            ErrorText::Full
        }
    }
}

pub(crate) fn set_error_text_from_env() {
    match std::env::var("CONCORD_ERROR_TEXT").as_deref() {
        Ok("own") => set_error_text(ErrorText::Own),
        Ok("full") | Err(_) => set_error_text(ErrorText::Full),
        Ok(_) => {
            UNKNOWN_TEXT_WARNING
                .call_once(|| tracing::warn!("unknown CONCORD_ERROR_TEXT; using full"));
            set_error_text(ErrorText::Full);
        }
    }
}

/// Emit a coded error record without recording its Display representation.
#[macro_export]
macro_rules! error {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let error: &dyn $crate::__private::coded_error::CodedError = &*error;
        let mut chain = ::std::vec![
            $crate::__private::coded_error::ChainLink {
                code: Some(error.code()),
                class: Some(error.class()),
                type_name: Some(error.type_name()),
                origin: $crate::__private::coded_error::Origin::Own,
                message: ::std::string::ToString::to_string(error),
            },
        ];
        if let Some(source) = error.source() {
            chain.extend($crate::__private::coded_error::chain(source));
        }
        $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
    }};
    ($err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let error: &dyn $crate::__private::coded_error::CodedError = &*error;
        let mut chain = ::std::vec![
            $crate::__private::coded_error::ChainLink {
                code: Some(error.code()),
                class: Some(error.class()),
                type_name: Some(error.type_name()),
                origin: $crate::__private::coded_error::Origin::Own,
                message: ::std::string::ToString::to_string(error),
            },
        ];
        if let Some(source) = error.source() {
            chain.extend($crate::__private::coded_error::chain(source));
        }
        $crate::__emit_error!(facts, chain, ::core::module_path!() $(, $($fields)*)?)
    }};
}

/// Emit a record for an anyhow error, using its nearest attached coded facts.
#[macro_export]
macro_rules! error_anyhow {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::coded_error::chain_anyhow(error);
        $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
    }};
    ($err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::coded_error::chain_anyhow(error);
        $crate::__emit_error!(facts, chain, ::core::module_path!() $(, $($fields)*)?)
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __emit_error {
    ($facts:expr, $chain:expr, $source:expr $(, $($fields:tt)*)?) => {{
        let facts: $crate::__private::coded_error::Facts = $facts;
        let chain: ::std::vec::Vec<$crate::__private::coded_error::ChainLink> = $chain;
        let source: &str = $source;
        let mode = $crate::error_text();
        let message = chain.iter()
            .find(|link| link.origin == $crate::__private::coded_error::Origin::Own)
            .or_else(|| if matches!(mode, $crate::ErrorText::Full) { chain.first() } else { None })
            .map(|link| link.message.as_str()).unwrap_or("");
        let chain_json = $crate::__private::serde_json::to_string(
            &chain.iter().map(|link| {
                let mut value = $crate::__private::serde_json::json!({
                    "code": link.code,
                    "class": link.class.map(|class| class.as_str()),
                    "type": link.type_name,
                    "origin": match link.origin {
                        $crate::__private::coded_error::Origin::Own => "own",
                        $crate::__private::coded_error::Origin::External => "external",
                    },
                });
                if matches!(mode, $crate::ErrorText::Full)
                    || link.origin == $crate::__private::coded_error::Origin::Own
                {
                    value["message"] = $crate::__private::serde_json::Value::String(link.message.clone());
                }
                value
            }).collect::<::std::vec::Vec<_>>()
        ).expect("error chain is serializable");
        let current = $crate::__private::tracing::Span::current();
        use $crate::__private::tracing_opentelemetry::OpenTelemetrySpanExt as _;
        current.set_attribute("error.code", facts.code);
        current.set_attribute("error.class", facts.class.as_str());
        current.set_status($crate::__private::opentelemetry::trace::Status::Error { description: "error".into() });
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
            error.chain = chain_json.as_str(),
            error.message = message,
            $($($fields)*)?
        );
    }};
}
