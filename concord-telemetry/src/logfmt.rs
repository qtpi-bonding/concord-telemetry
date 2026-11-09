//! Trace correlation for logs.
//!
//! Local lines: `TraceFormat` wraps tracing-subscriber's stock formatter and,
//! only when an exporter is configured, adds `trace_id`, `span_id` and
//! `baggage` to a JSON line and ` trace=<first 8 hex>` to a text line. The JSON
//! line is spliced rather than parsed and re-serialized: re-serializing would
//! reorder the fields, and the stock line is a flat object that ends in `}`.
//!
//! Exported records: the OpenTelemetry bridge builds and emits its record
//! itself, so `BaggageBridge` attaches the span's baggage to the current
//! `Context` around the bridge, and `BaggageLogProcessor` copies that baggage
//! into the record's attributes on its way to the real processor. The same
//! two add the code location (see `location`).

use crate::baggage::BaggageMap;
use opentelemetry::baggage::BaggageExt;
use opentelemetry::logs::{AnyValue, LogRecord as _};
use opentelemetry::trace::TraceContextExt;
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::logs::{LogProcessor, SdkLogRecord};
use opentelemetry_sdk::Resource;
use std::fmt;
use std::time::Duration;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry::LookupSpan;

/// The trace and span ids of the current span, when it has a valid context.
/// `Span::current()` sees the subscriber only when it is the global default:
/// tracing hides a scoped one from code running inside its own dispatch.
fn current_ids() -> Option<(String, String)> {
    let context = Span::current().context();
    let span = context.span();
    let span_context = span.span_context();
    span_context.is_valid().then(|| {
        (
            span_context.trace_id().to_string(),
            span_context.span_id().to_string(),
        )
    })
}

/// A formatter that adds trace fields to the stock formatter's line while
/// `active` returns true.
pub(crate) struct TraceFormat<F> {
    inner: F,
    json: bool,
    active: fn() -> bool,
}

impl<F> TraceFormat<F> {
    pub(crate) fn json(inner: F, active: fn() -> bool) -> Self {
        Self {
            inner,
            json: true,
            active,
        }
    }

    pub(crate) fn text(inner: F, active: fn() -> bool) -> Self {
        Self {
            inner,
            json: false,
            active,
        }
    }
}

impl<S, N, F> FormatEvent<S, N> for TraceFormat<F>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        if !(self.active)() {
            return self.inner.format_event(ctx, writer.by_ref(), event);
        }
        let Some((trace_id, span_id)) = current_ids() else {
            return self.inner.format_event(ctx, writer.by_ref(), event);
        };
        if !self.json {
            // The inner formatter keeps its own colour setting (`Format::with_ansi`);
            // the adapter only holds back the final newline for the suffix.
            let mut held = HoldNewline {
                out: &mut writer,
                held: false,
            };
            self.inner
                .format_event(ctx, Writer::new(&mut held), event)?;
            return held.finish(&format!(" trace={}", &trace_id[..8]));
        }
        let mut line = String::new();
        self.inner
            .format_event(ctx, Writer::new(&mut line), event)?;
        let body = line.strip_suffix('\n').unwrap_or(&line);
        let Some(body) = body.strip_suffix('}') else {
            return writer.write_str(&line);
        };
        let entries = ctx
            .event_scope()
            .and_then(|mut scope| scope.next())
            .and_then(|span| span.extensions().get::<BaggageMap>().cloned())
            .unwrap_or_default();
        let baggage: serde_json::Map<String, serde_json::Value> = entries
            .0
            .into_iter()
            .map(|(key, value)| (key, serde_json::Value::String(value)))
            .collect();
        let baggage = serde_json::Value::Object(baggage);
        writeln!(
            writer,
            "{body},\"trace_id\":\"{trace_id}\",\"span_id\":\"{span_id}\",\"baggage\":{baggage}}}"
        )
    }
}

/// Passes writes through, holding back a trailing newline so a suffix can go
/// before it.
struct HoldNewline<'a, 'w> {
    out: &'a mut Writer<'w>,
    held: bool,
}

impl HoldNewline<'_, '_> {
    fn finish(self, suffix: &str) -> fmt::Result {
        self.out.write_str(suffix)?;
        if self.held {
            self.out.write_char('\n')?;
        }
        Ok(())
    }
}

impl fmt::Write for HoldNewline<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.is_empty() {
            return Ok(());
        }
        if self.held {
            self.out.write_char('\n')?;
            self.held = false;
        }
        match text.strip_suffix('\n') {
            Some(rest) => {
                self.out.write_str(rest)?;
                self.held = true;
                Ok(())
            }
            None => self.out.write_str(text),
        }
    }
}

/// Wraps the OpenTelemetry log bridge: while the bridge builds a record, the
/// event's span baggage is the current context's baggage.
pub(crate) struct BaggageBridge<L>(pub L);

impl<S, L> Layer<S> for BaggageBridge<L>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    L: Layer<S>,
{
    fn on_layer(&mut self, subscriber: &mut S) {
        self.0.on_layer(subscriber);
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: LayerContext<'_, S>,
    ) {
        self.0.on_new_span(attrs, id, ctx);
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: LayerContext<'_, S>,
    ) {
        self.0.on_record(id, values, ctx);
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: LayerContext<'_, S>) {
        let entries = ctx.event_span(event).and_then(|span| {
            span.extensions()
                .get::<BaggageMap>()
                .map(|map| map.0.clone())
        });
        // The extensions lock is released before the bridge runs.
        crate::location::with_event_location(event.metadata(), || {
            match entries.filter(|entries| !entries.is_empty()) {
                Some(entries) => {
                    let baggage = entries
                        .into_iter()
                        .map(|(key, value)| KeyValue::new(key, value));
                    let _guard = Context::current().with_baggage(baggage).attach();
                    self.0.on_event(event, ctx);
                }
                None => self.0.on_event(event, ctx),
            }
        });
    }
}

/// Copies the current context's baggage into each record's attributes, then
/// hands the record to the wrapped processor.
#[derive(Debug)]
pub(crate) struct BaggageLogProcessor<P>(pub P);

impl<P: LogProcessor> LogProcessor for BaggageLogProcessor<P> {
    fn emit(
        &self,
        record: &mut SdkLogRecord,
        instrumentation: &opentelemetry::InstrumentationScope,
    ) {
        let baggage: Vec<_> = Context::map_current(|context| {
            context
                .baggage()
                .iter()
                .map(|(key, (value, _))| (key.clone(), value.to_string()))
                .collect()
        });
        for (key, value) in baggage {
            // The event's own field of the same name wins.
            if !record.attributes_iter().any(|(own, _)| *own == key) {
                record.add_attribute(key, AnyValue::from(value));
            }
        }
        if let Some(location) = crate::location::current() {
            let mut add = |key: &'static str, value: AnyValue| {
                if !record.attributes_iter().any(|(own, _)| own.as_str() == key) {
                    record.add_attribute(key, value);
                }
            };
            if let Some(file) = location.file {
                add("code.file.path", AnyValue::from(file));
            }
            if let Some(line) = location.line {
                add("code.line.number", AnyValue::from(i64::from(line)));
            }
            if let Some(module) = location.module {
                add("code.module.name", AnyValue::from(module));
            }
        }
        self.0.emit(record, instrumentation);
    }

    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.shutdown()
    }

    fn event_enabled(
        &self,
        level: opentelemetry::logs::Severity,
        target: &str,
        name: Option<&str>,
    ) -> bool {
        self.0.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("buffer").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Buffer {
            self.clone()
        }
    }

    fn on() -> bool {
        true
    }

    fn off() -> bool {
        false
    }

    /// A text line through `TraceFormat`, with the layer's own ANSI setting
    /// `layer_ansi` and the inner format's `format_ansi`.
    fn text_line(active: fn() -> bool, layer_ansi: bool, format_ansi: Option<bool>) -> String {
        let buffer = Buffer::default();
        let mut inner = tracing_subscriber::fmt::format();
        if let Some(ansi) = format_ansi {
            inner = inner.with_ansi(ansi);
        }
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(layer_ansi)
                .with_writer(buffer.clone())
                .event_format(TraceFormat::text(inner, active)),
        );
        tracing::subscriber::with_default(subscriber, || tracing::info!("hello"));
        let bytes = buffer.0.lock().expect("buffer").clone();
        String::from_utf8(bytes).expect("utf8")
    }

    #[test]
    fn text_keeps_ansi_when_inactive() {
        assert!(text_line(off, true, None).contains('\u{1b}'));
        assert!(!text_line(off, false, None).contains('\u{1b}'));
    }

    #[test]
    fn text_keeps_ansi_through_the_active_path() {
        // A scoped subscriber hides the span from `Span::current()`, so this
        // takes the delegate path; the ids path is covered by text_trace_suffix.
        assert!(text_line(on, true, None).contains('\u{1b}'));
    }

    #[test]
    fn the_inner_format_colours_independently_of_the_writer() {
        assert!(text_line(off, false, Some(true)).contains('\u{1b}'));
    }

    #[test]
    fn hold_newline_puts_the_suffix_before_the_final_newline_only() {
        let mut out = String::new();
        let mut writer = Writer::new(&mut out);
        let mut held = HoldNewline {
            out: &mut writer,
            held: false,
        };
        held.write_str("a\nb").expect("write");
        held.write_str("c\n").expect("write");
        held.finish(" trace=x").expect("finish");
        assert_eq!(out, "a\nbc trace=x\n");
    }
}
