use crate::coded::emit_coded;
#[cfg(feature = "otel")]
use crate::deadline::{flush_providers, run_exclusive, Gate};
use coded_error::ErrorClass;
#[cfg(feature = "otel")]
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};
use std::sync::OnceLock;
#[cfg(feature = "otel")]
use std::sync::{Mutex, PoisonError};
#[cfg(feature = "otel")]
use std::time::Duration;

pub const PANIC_CODE: &str = "concord.process.panic";

type Hook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

/// The hook that was installed before ours. It is taken once and chained once,
/// however many times `init` runs.
static PREVIOUS: OnceLock<Hook> = OnceLock::new();

/// A flush started by a panic. At most one is in flight: a panic while it
/// runs waits for it, within the deadline, and then flushes its own record.
#[cfg(feature = "otel")]
static FLUSHING: Gate = Gate::new();

/// What the hook flushes: the providers of the latest `init`.
#[cfg(feature = "otel")]
struct Target {
    logs: Option<SdkLoggerProvider>,
    traces: Option<SdkTracerProvider>,
    deadline: Duration,
}

#[cfg(feature = "otel")]
static TARGET: Mutex<Option<Target>> = Mutex::new(None);

/// Installs the panic hook the first time it is called and points it at these
/// providers; a later call only swaps the providers. Each panic is recorded,
/// then both providers are flushed for at most `deadline`
/// (`Config::flush_deadline`).
#[cfg(feature = "otel")]
pub(crate) fn install(
    logs: Option<SdkLoggerProvider>,
    traces: Option<SdkTracerProvider>,
    deadline: Duration,
) {
    *TARGET.lock().unwrap_or_else(PoisonError::into_inner) = Some(Target {
        logs,
        traces,
        deadline,
    });
    install_hook();
}

/// Stops the hook flushing providers that `Guard` is shutting down.
#[cfg(feature = "otel")]
pub(crate) fn release() {
    *TARGET.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Installs the panic hook. Each panic is recorded; there is no exporter to
/// flush.
#[cfg(not(feature = "otel"))]
pub(crate) fn install() {
    install_hook();
}

/// Chains the recording hook in front of the previous one, once.
fn install_hook() {
    let mut first = false;
    PREVIOUS.get_or_init(|| {
        first = true;
        std::panic::take_hook()
    });
    if first {
        std::panic::set_hook(Box::new(on_panic));
    }
}

/// Records the panic, flushes (`otel` build), then runs the previous hook.
fn on_panic(info: &std::panic::PanicHookInfo<'_>) {
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
        Some(location) => crate::location::emit_at_owned(location.file(), location.line(), emit),
        None => emit(),
    }

    #[cfg(feature = "otel")]
    flush_target();
    if let Some(previous) = PREVIOUS.get() {
        previous(info);
    }
}

#[cfg(feature = "otel")]
fn flush_target() {
    let (logs, traces, deadline) = {
        let guard = TARGET.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some(target) => (target.logs.clone(), target.traces.clone(), target.deadline),
            None => return,
        }
    };
    if logs.is_some() || traces.is_some() {
        run_exclusive(&FLUSHING, deadline, move || {
            flush_providers(logs.as_ref(), traces.as_ref());
        });
    }
}
