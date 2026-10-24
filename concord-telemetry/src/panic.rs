use crate::{deadline::run_with_deadline, EVENT_TARGET};
use coded_error::{fingerprint, ErrorClass};
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};
use std::time::Duration;

pub const PANIC_CODE: &str = "concord.process.panic";
const PANIC_FLUSH: Duration = Duration::from_secs(2);

pub(crate) fn install(logs: Option<SdkLoggerProvider>, traces: Option<SdkTracerProvider>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let source = info
            .location()
            .map(|location| location.file())
            .unwrap_or("unknown");
        let class = ErrorClass::Unknown;
        let digest = fingerprint(PANIC_CODE, class, source);
        tracing::event!(
            name: "error",
            target: EVENT_TARGET,
            tracing::Level::ERROR,
            error.code = PANIC_CODE,
            error.class = class.as_str(),
            error.fingerprint = digest.as_str(),
            error.source = source,
        );

        let log_provider = logs.clone();
        let trace_provider = traces.clone();
        if log_provider.is_some() || trace_provider.is_some() {
            run_with_deadline(PANIC_FLUSH, move || {
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
