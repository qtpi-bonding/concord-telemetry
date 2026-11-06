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
//! into the record's attributes on its way to the real processor.

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
        let mut line = String::new();
        self.inner
            .format_event(ctx, Writer::new(&mut line), event)?;
        if !(self.active)() {
            return writer.write_str(&line);
        }
        let Some((trace_id, span_id)) = current_ids() else {
            return writer.write_str(&line);
        };
        let body = line.strip_suffix('\n').unwrap_or(&line);
        if self.json {
            let Some(body) = body.strip_suffix('}') else {
                return writer.write_str(&line);
            };
            let entries = ctx
                .lookup_current()
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
        } else {
            writeln!(writer, "{body} trace={}", &trace_id[..8])
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
        Context::map_current(|context| {
            for (key, (value, _)) in context.baggage().iter() {
                record.add_attribute(key.clone(), AnyValue::from(value.to_string()));
            }
        });
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
