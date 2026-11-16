use crate::coded::emit_coded;
#[cfg(feature = "otel")]
use crate::deadline::{run_exclusive, Gate};
use coded_error::ErrorClass;
#[cfg(feature = "otel")]
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};
#[cfg(feature = "otel")]
use std::time::Duration;

pub const PANIC_CODE: &str = "concord.process.panic";

/// A flush started by a panic. At most one is in flight: a panic while it
/// runs waits for it, within the deadline, and then flushes its own record.
#[cfg(feature = "otel")]
static FLUSHING: Gate = Gate::new();

/// Installs the panic hook. Each panic is recorded, then both providers are
/// flushed for at most `deadline` (`Config::flush_deadline`).
#[cfg(feature = "otel")]
pub(crate) fn install(
    logs: Option<SdkLoggerProvider>,
    traces: Option<SdkTracerProvider>,
    deadline: Duration,
) {
    install_hook(move || {
        let log_provider = logs.clone();
        let trace_provider = traces.clone();
        if log_provider.is_some() || trace_provider.is_some() {
            run_exclusive(&FLUSHING, deadline, move || {
                if let Some(provider) = log_provider {
                    let _ = provider.force_flush();
                }
                if let Some(provider) = trace_provider {
                    let _ = provider.force_flush();
                }
            });
        }
    });
}

/// Installs the panic hook. Each panic is recorded; there is no exporter to
/// flush.
#[cfg(not(feature = "otel"))]
pub(crate) fn install() {
    install_hook(|| {});
}

/// Chains the recording hook in front of the previous one; `flush` runs after
/// the record is made and before the previous hook.
fn install_hook(flush: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let source = info
            .location()
            .map(|location| location.file())
            .unwrap_or("unknown");
        let emit = || {
            emit_coded!(
                tracing::Level::ERROR,
                PANIC_CODE,
                ErrorClass::Unknown,
                source
            )
        };
        match info.location() {
            Some(location) => {
                crate::location::emit_at_owned(location.file(), location.line(), emit)
            }
            None => emit(),
        }

        flush();
        previous(info);
    }));
}
