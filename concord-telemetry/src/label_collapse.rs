//! The end-of-span processor that resolves labels.
//!
//! tracing-opentelemetry and the SDK append on every record and
//! `set_attribute`, so a key written twice is exported twice, and the OTel
//! spec says a repeated one SHOULD overwrite. `BaggageLayer` stamps labels
//! under `concord.label.<key>`; `LabelCollapse`, wrapped around the
//! exporting processor, resolves a finished span once:
//!
//! - every repeated key, bare or prefixed, collapses to its last value, and
//!   is never reported (re-records and repeated `error!` calls are normal);
//! - a `concord.label.<k>` whose span has its own `<k>` is dropped, and when
//!   the exported strings differ the key goes into the span attribute
//!   `concord.label_conflicts` (a string array of keys, never values);
//! - any other `concord.label.<k>` is renamed to `<k>`.
//!
//! A span with no label key and no repeated key passes through untouched,
//! without allocating. Unsampled spans always do. A custom tracer provider
//! must install the wrapper: `with_span_processor(LabelCollapse(..))`.
//!
//! Nothing is logged here: a WARN from `on_end` would attach to whatever span
//! is current, inside tracing-opentelemetry's `on_close`.

use crate::baggage::LABEL_PREFIX;
use opentelemetry::{Array, Context, Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};
use opentelemetry_sdk::Resource;
use std::collections::HashSet;
use std::time::Duration;

/// The span attribute naming the label keys whose inherited value differed
/// from the span's own.
pub(crate) const CONFLICTS_KEY: &str = "concord.label_conflicts";

/// The provider's `max_attributes_per_span`. Labels are stamped on every
/// change and only collapsed when the span ends, and the SDK's default of 128
/// would drop later attributes (a status code recorded last) first. The
/// collapse cannot recover attributes the SDK already dropped.
pub(crate) const MAX_SPAN_ATTRIBUTES: u32 = 512;

/// Wraps the exporting span processor and collapses labels in `on_end`.
#[derive(Debug)]
pub struct LabelCollapse<P>(pub P);

impl<P: SpanProcessor> SpanProcessor for LabelCollapse<P> {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.0.on_start(span, cx);
    }

    fn on_end(&self, mut span: SpanData) {
        if span.span_context.is_sampled() {
            collapse(&mut span.attributes);
        }
        self.0.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.0.shutdown()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

/// True when a key is repeated or carries the label prefix.
fn needs_work(attributes: &[KeyValue]) -> bool {
    attributes.iter().enumerate().any(|(i, kv)| {
        kv.key.as_str().starts_with(LABEL_PREFIX)
            || attributes[i + 1..].iter().any(|later| later.key == kv.key)
    })
}

pub(crate) fn collapse(attributes: &mut Vec<KeyValue>) {
    if !needs_work(attributes) {
        return;
    }
    // Keep the last occurrence of every key, in order.
    let mut seen: HashSet<Key> = HashSet::with_capacity(attributes.len());
    let mut last: Vec<KeyValue> = std::mem::take(attributes)
        .into_iter()
        .rev()
        .filter(|kv| seen.insert(kv.key.clone()))
        .collect();
    last.reverse();

    let mut conflicts: Vec<String> = Vec::new();
    let mut resolved = Vec::with_capacity(last.len() + 1);
    for kv in &last {
        let Some(bare) = kv.key.as_str().strip_prefix(LABEL_PREFIX) else {
            resolved.push(kv.clone());
            continue;
        };
        // Never a label's own key: it would overwrite the conflicts report.
        if bare.is_empty() || bare == CONFLICTS_KEY {
            continue;
        }
        match last.iter().find(|own| own.key.as_str() == bare) {
            Some(own) => {
                if own.value.as_str() != kv.value.as_str() {
                    conflicts.push(bare.to_owned());
                }
            }
            None if bare.is_empty() => {}
            None => resolved.push(KeyValue::new(bare.to_owned(), kv.value.clone())),
        }
    }
    if !conflicts.is_empty() {
        resolved.retain(|kv| kv.key.as_str() != CONFLICTS_KEY);
        conflicts.sort();
        conflicts.dedup();
        let keys: Vec<StringValue> = conflicts.into_iter().map(StringValue::from).collect();
        resolved.push(KeyValue::new(
            CONFLICTS_KEY,
            Value::Array(Array::String(keys)),
        ));
    }
    *attributes = resolved;
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{Tracer, TracerProvider};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Default)]
    struct Calls(Arc<Mutex<Vec<String>>>);

    impl Calls {
        fn note(&self, call: &str) {
            self.0.lock().unwrap().push(call.to_owned());
        }
        fn seen(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    #[derive(Debug)]
    struct Recorder(Calls);

    impl SpanProcessor for Recorder {
        fn on_start(&self, _: &mut Span, _: &Context) {
            self.0.note("on_start");
        }
        fn on_end(&self, span: SpanData) {
            self.0.note(&format!("on_end:{}", span.attributes.len()));
        }
        fn force_flush(&self) -> OTelSdkResult {
            self.0.note("force_flush");
            Ok(())
        }
        fn shutdown_with_timeout(&self, _: Duration) -> OTelSdkResult {
            self.0.note("shutdown_with_timeout");
            Ok(())
        }
        fn shutdown(&self) -> OTelSdkResult {
            self.0.note("shutdown");
            Ok(())
        }
        fn set_resource(&mut self, resource: &Resource) {
            let name = resource
                .iter()
                .find(|(key, _)| key.as_str() == "service.name")
                .map(|(_, value)| value.to_string())
                .unwrap_or_default();
            self.0.note(&format!("set_resource:{name}"));
        }
    }

    fn provider(calls: &Calls) -> SdkTracerProvider {
        SdkTracerProvider::builder()
            .with_resource(Resource::builder_empty().with_service_name("svc").build())
            .with_span_processor(LabelCollapse(Recorder(calls.clone())))
            .build()
    }

    #[test]
    fn the_wrapper_forwards_set_resource() {
        let calls = Calls::default();
        let _provider = provider(&calls);
        assert!(
            calls.seen().contains(&"set_resource:svc".to_owned()),
            "{:?}",
            calls.seen()
        );
    }

    #[test]
    fn the_wrapper_forwards_on_start_on_end_and_flush() {
        let calls = Calls::default();
        let provider = provider(&calls);
        let tracer = provider.tracer("t");
        let mut span = tracer.start("s");
        opentelemetry::trace::Span::set_attribute(&mut span, KeyValue::new("a", 1));
        opentelemetry::trace::Span::set_attribute(&mut span, KeyValue::new("a", 2));
        drop(span);
        provider.force_flush().unwrap();
        let seen = calls.seen();
        assert!(seen.contains(&"on_start".to_owned()), "{seen:?}");
        // The repeated key collapsed before the inner processor saw it.
        assert!(seen.contains(&"on_end:1".to_owned()), "{seen:?}");
        assert!(seen.contains(&"force_flush".to_owned()), "{seen:?}");
    }

    #[test]
    fn the_wrapper_forwards_shutdown_and_shutdown_with_timeout() {
        let calls = Calls::default();
        let wrapper = LabelCollapse(Recorder(calls.clone()));
        wrapper.shutdown().unwrap();
        wrapper
            .shutdown_with_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(calls.seen(), ["shutdown", "shutdown_with_timeout"]);
    }

    fn kv(key: &'static str, value: &str) -> KeyValue {
        KeyValue::new(key, value.to_owned())
    }

    #[test]
    fn a_span_without_labels_or_repeats_is_untouched() {
        let mut attributes = vec![kv("a", "1"), kv("b", "2")];
        let before = attributes.clone();
        collapse(&mut attributes);
        assert_eq!(attributes, before);
    }

    #[test]
    fn a_repeated_key_keeps_its_last_value_and_labels_resolve() {
        let mut attributes = vec![
            kv("concord.label.action_id", "A"),
            kv("concord.label.turn_id", "T"),
            kv("turn_id", "T"),
            kv("x", "1"),
            kv("x", "2"),
            kv("concord.label.request_id", "caller"),
            kv("request_id", "own"),
            kv("concord.label.agent_id", "G1"),
            kv("concord.label.agent_id", "G2"),
        ];
        collapse(&mut attributes);
        let get = |key: &str| {
            attributes
                .iter()
                .find(|kv| kv.key.as_str() == key)
                .map(|kv| kv.value.to_string())
        };
        assert_eq!(get("action_id").as_deref(), Some("A"));
        assert_eq!(get("turn_id").as_deref(), Some("T"));
        assert_eq!(get("x").as_deref(), Some("2"));
        assert_eq!(get("request_id").as_deref(), Some("own"));
        assert_eq!(get("agent_id").as_deref(), Some("G2"));
        assert_eq!(get(CONFLICTS_KEY).as_deref(), Some(r#"["request_id"]"#));
        assert!(attributes
            .iter()
            .all(|kv| !kv.key.as_str().starts_with(LABEL_PREFIX)));
        assert_eq!(attributes.len(), 6);
    }

    #[test]
    fn a_label_never_renames_onto_the_conflicts_key() {
        // A clean span: the label that would become concord.label_conflicts is dropped.
        let mut clean = vec![
            kv("concord.label.concord.label_conflicts", "x"),
            kv("a", "1"),
        ];
        collapse(&mut clean);
        assert!(
            clean.iter().all(|kv| kv.key.as_str() != CONFLICTS_KEY),
            "{clean:?}"
        );

        // A clashing span: exactly one conflicts key, listing only the real clash,
        // even when the span already has one of its own.
        let mut clash = vec![
            kv("concord.label_conflicts", "own"),
            kv("concord.label.concord.label_conflicts", "x"),
            kv("concord.label.request_id", "caller"),
            kv("request_id", "own"),
        ];
        collapse(&mut clash);
        let found: Vec<String> = clash
            .iter()
            .filter(|kv| kv.key.as_str() == CONFLICTS_KEY)
            .map(|kv| kv.value.to_string())
            .collect();
        assert_eq!(found, [r#"["request_id"]"#.to_owned()]);
    }
}
