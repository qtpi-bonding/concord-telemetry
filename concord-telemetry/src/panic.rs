use crate::coded::emit_coded;
use crate::deadline::run_exclusive;
use coded_error::ErrorClass;
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const PANIC_CODE: &str = "concord.process.panic";

/// A flush started by a panic. At most one is in flight: a panic while it
/// runs does not start another, and one after it finishes does.
static FLUSHING: AtomicBool = AtomicBool::new(false);

/// Installs the panic hook. Each panic is recorded, then both providers are
/// flushed for at most `deadline` (`Config::flush_deadline`).
pub(crate) fn install(
    logs: Option<SdkLoggerProvider>,
    traces: Option<SdkTracerProvider>,
    deadline: Duration,
) {
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
        previous(info);
    }));
}
