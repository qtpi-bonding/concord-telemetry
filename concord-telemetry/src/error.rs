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
        if matches!(t, ErrorText::Own) { 0 } else { 1 },
        Ordering::Relaxed,
    );
}

pub fn error_text() -> ErrorText {
    if ERROR_TEXT.load(Ordering::Relaxed) == 0 {
        ErrorText::Own
    } else {
        ErrorText::Full
    }
}

/// Reads the environment without emitting a warning. The returned value is
/// logged by `init` after its subscriber has been installed.
#[doc(hidden)]
pub fn set_error_text_from_env() -> Option<String> {
    match std::env::var("CONCORD_ERROR_TEXT") {
        Ok(value) if value == "own" => {
            set_error_text(ErrorText::Own);
            None
        }
        Ok(value) if value == "full" => {
            set_error_text(ErrorText::Full);
            None
        }
        Ok(value) => {
            set_error_text(ErrorText::Full);
            Some(value)
        }
        Err(_) => None,
    }
}

#[doc(hidden)]
pub fn warn_unknown_error_text(value: &str) {
    set_error_text(ErrorText::Full);
    UNKNOWN_TEXT_WARNING
        .call_once(|| tracing::warn!("unknown CONCORD_ERROR_TEXT value {:?}; using full", value));
}

// Kept as ordinary functions so macro expansions contain no chain or JSON logic.
pub fn chain_typed(error: &dyn coded_error::CodedError) -> Vec<coded_error::ChainLink> {
    let mut links = vec![coded_error::ChainLink {
        code: Some(error.code()),
        class: Some(error.class()),
        type_name: Some(error.type_name()),
        origin: coded_error::Origin::Own,
        message: error.to_string(),
    }];
    if let Some(source) = error.source() {
        links.extend(coded_error::chain(source));
    }
    links
}

pub fn chain_anyhow(error: &anyhow::Error) -> Vec<coded_error::ChainLink> {
    coded_error::chain_anyhow(error)
}

pub fn chain_json(links: &[coded_error::ChainLink], mode: ErrorText) -> (String, Option<String>) {
    let message = links
        .iter()
        .find(|link| link.origin == coded_error::Origin::Own)
        .or_else(|| {
            if matches!(mode, ErrorText::Full) {
                links.first()
            } else {
                None
            }
        })
        .map(|link| link.message.clone());
    let json = serde_json::to_string(&links.iter().map(|link| {
        let mut value = serde_json::json!({
            "code": link.code, "class": link.class.map(|class| class.as_str()),
            "type": link.type_name,
            "origin": match link.origin { coded_error::Origin::Own => "own", coded_error::Origin::External => "external" },
        });
        if matches!(mode, ErrorText::Full) || link.origin == coded_error::Origin::Own {
            value["message"] = serde_json::Value::String(link.message.clone());
        }
        value
    }).collect::<Vec<_>>()).expect("error chain is serializable");
    (json, message)
}

/// Emit a coded error record without recording its Display representation.
#[macro_export]
macro_rules! error {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let chain = $crate::__private::error::chain_typed(&*error);
        $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
    }};
    ($err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let chain = $crate::__private::error::chain_typed(&*error);
        $crate::__emit_error!(facts, chain, ::core::module_path!() $(, $($fields)*)?)
    }};
}

#[macro_export]
macro_rules! error_anyhow {
    ($err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::error::chain_anyhow(error);
        $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
    }};
    ($err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::error::chain_anyhow(error);
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
        let (chain_json, message) = $crate::__private::error::chain_json(&chain, $crate::error_text());
        let current = $crate::__private::tracing::Span::current();
        use $crate::__private::tracing_opentelemetry::OpenTelemetrySpanExt as _;
        current.set_attribute("error.code", facts.code);
        current.set_attribute("error.class", facts.class.as_str());
        current.set_status($crate::__private::opentelemetry::trace::Status::Error { description: "error".into() });
        let fingerprint = $crate::__private::coded_error::fingerprint(facts.code, facts.class, source);
        $crate::__private::tracing::event!(
            name: "error", target: $crate::EVENT_TARGET, $crate::__private::tracing::Level::ERROR,
            error.code = facts.code, error.class = facts.class.as_str(),
            error.fingerprint = fingerprint.as_str(), error.source = source,
            error.chain = chain_json.as_str(), error.message = message.as_deref().unwrap_or(""),
            $($($fields)*)?
        );
    }};
}
