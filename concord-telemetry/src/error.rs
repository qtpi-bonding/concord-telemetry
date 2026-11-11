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
    UNKNOWN_TEXT_WARNING.call_once(|| {
        tracing::warn!(
            error_text = value,
            "unknown CONCORD_ERROR_TEXT value; using full"
        )
    });
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

/// Cap on the text of one error record: `error.message` and the serialized
/// `error.chain` each stay within half of it, 8192 bytes together.
pub const ERROR_TEXT_MAX: usize = 8192;
const TRUNCATED: &str = "...[truncated]";

/// `text` cut to at most `max` bytes on a character boundary, ending in a
/// visible marker when anything was cut.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max.saturating_sub(TRUNCATED.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_owned();
    out.push_str(&TRUNCATED[..TRUNCATED.len().min(max)]);
    out
}

fn links_json(links: &[coded_error::ChainLink], mode: ErrorText, cap: usize) -> String {
    serde_json::to_string(&links.iter().map(|link| {
        let mut value = serde_json::json!({
            "code": link.code, "class": link.class.map(|class| class.as_str()),
            "type": link.type_name,
            "origin": match link.origin { coded_error::Origin::Own => "own", coded_error::Origin::External => "external" },
        });
        if matches!(mode, ErrorText::Full) || link.origin == coded_error::Origin::Own {
            value["message"] = serde_json::Value::String(truncate(&link.message, cap));
        }
        value
    }).collect::<Vec<_>>()).expect("error chain is serializable")
}

pub fn chain_json(links: &[coded_error::ChainLink], mode: ErrorText) -> (String, Option<String>) {
    let half = ERROR_TEXT_MAX / 2;
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
        .map(|link| truncate(&link.message, half));
    // Shrink the per-link text until the whole chain fits; codes, classes and
    // types are never cut, so a chain whose metadata alone is over the cap
    // is kept as is.
    let mut cap = half;
    let mut json = links_json(links, mode, cap);
    while json.len() > half && cap > 0 {
        cap /= 2;
        json = links_json(links, mode, cap);
    }
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

/// Records `error` with an explicit code location and runtime field pairs, for
/// a wrapper (an error hook) whose field names are only known at run time, which
/// `tracing::event!` cannot take. The record is the one `error_at!` makes (the
/// fingerprint source is `module`), and each pair is added as a record attribute
/// of its own type: an `AttrValue::I64` stays an integer. A pair whose name the
/// record already has is skipped. The names on the slice are not checked by
/// `concord-lint`; the wrapper's own call sites are, once the wrapper is
/// declared in `[lint] log_macros`.
pub fn emit_error_at(
    file: &'static str,
    line: u32,
    module: &'static str,
    error: &dyn coded_error::CodedError,
    fields: &[(&'static str, crate::AttrValue)],
) {
    static NEVER: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    let never = NEVER.get_or_init(|| concord_fields::shared().never);
    let mut kept = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        if never.contains(*name) {
            crate::carrier::emit_field_dropped(name);
        } else {
            kept.push((*name, value.clone()));
        }
    }
    let facts = coded_error::Facts::of(error);
    let chain = chain_typed(error);
    crate::emit_at(file, line, module, || {
        crate::location::with_extra(&kept, || {
            crate::__emit_error!(facts, chain, module);
        });
    });
}

/// `error!` with an explicit code location: the record's `code.file.path`,
/// `code.line.number` and `code.module.name` are `file`, `line` and `module`
/// (a `&'static str`, a `u32` and a `&'static str`) instead of the macro's own
/// call site, and `module` is the fingerprint source unless `source =` is given.
/// For a crate that records on behalf of its callers, such as an error hook
/// receiving the location from a macro in the code that failed:
///
/// ```ignore
/// concord_telemetry::error_at!(file!(), line!(), module_path!(), &err, runtime_session_id = id);
/// ```
///
/// Field values are evaluated inside a closure, so `.await` and `?` do not work
/// in them.
#[macro_export]
macro_rules! error_at {
    ($file:expr, $line:expr, $module:expr, $err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let chain = $crate::__private::error::chain_typed(&*error);
        $crate::emit_at($file, $line, $module, || {
            $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
        })
    }};
    ($file:expr, $line:expr, $module:expr, $err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::Facts::of(error);
        let chain = $crate::__private::error::chain_typed(&*error);
        let module: &'static str = $module;
        $crate::emit_at($file, $line, module, || {
            $crate::__emit_error!(facts, chain, module $(, $($fields)*)?)
        })
    }};
}

/// `error_at!` for an `anyhow::Error`, as `error_anyhow!` is to `error!`.
#[macro_export]
macro_rules! error_at_anyhow {
    ($file:expr, $line:expr, $module:expr, $err:expr, source = $source:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::error::chain_anyhow(error);
        $crate::emit_at($file, $line, $module, || {
            $crate::__emit_error!(facts, chain, $source $(, $($fields)*)?)
        })
    }};
    ($file:expr, $line:expr, $module:expr, $err:expr $(, $($fields:tt)*)?) => {{
        let error = $err;
        let facts = $crate::__private::coded_error::facts_anyhow(error);
        let chain = $crate::__private::error::chain_anyhow(error);
        let module: &'static str = $module;
        $crate::emit_at($file, $line, module, || {
            $crate::__emit_error!(facts, chain, module $(, $($fields)*)?)
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn link(origin: coded_error::Origin, message: String) -> coded_error::ChainLink {
        coded_error::ChainLink {
            code: Some("test.code"),
            class: Some(coded_error::ErrorClass::Logical),
            type_name: Some("T"),
            origin,
            message,
        }
    }

    #[test]
    fn short_text_is_untouched() {
        let links = [link(coded_error::Origin::Own, "short".into())];
        let (json, message) = chain_json(&links, ErrorText::Full);
        assert_eq!(message.as_deref(), Some("short"));
        assert!(json.contains("\"message\":\"short\""));
    }

    #[test]
    fn long_text_is_cut_on_a_char_boundary_with_a_marker() {
        // 3-byte characters, so a cut at an arbitrary byte lands mid-character.
        let long = "\u{20ac}".repeat(10_000);
        let links = [
            link(coded_error::Origin::Own, long.clone()),
            link(coded_error::Origin::External, long),
        ];
        let (json, message) = chain_json(&links, ErrorText::Full);
        let message = message.unwrap();
        assert!(message.len() <= ERROR_TEXT_MAX / 2);
        assert!(message.ends_with(TRUNCATED));
        assert!(json.len() <= ERROR_TEXT_MAX / 2, "{}", json.len());
        assert!(json.len() + message.len() <= ERROR_TEXT_MAX);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        for entry in parsed.as_array().unwrap() {
            assert_eq!(entry["code"], "test.code");
            assert_eq!(entry["class"], "logical");
            assert!(entry["message"].as_str().unwrap().ends_with(TRUNCATED));
        }
    }

    #[test]
    fn truncate_never_exceeds_max() {
        for max in [0, 1, 5, 14, 15, 40] {
            let out = truncate(&"\u{20ac}".repeat(100), max);
            assert!(out.len() <= max, "{max}: {}", out.len());
        }
    }
}
