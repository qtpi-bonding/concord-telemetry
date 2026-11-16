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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[cfg(feature = "otel")]
    fn install_with(seconds: u64) {
        install(None, None, Duration::from_secs(seconds));
    }

    #[cfg(not(feature = "otel"))]
    fn install_with(_seconds: u64) {
        install();
    }

    /// Two installs (the second with different providers and deadline) chain
    /// the previous hook once: it runs once per panic, not twice.
    #[test]
    fn second_install_does_not_chain_the_hook_again() {
        let probe = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&probe);
        std::panic::set_hook(Box::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        }));

        install_with(1);
        install_with(7);

        #[cfg(feature = "otel")]
        {
            let target = TARGET.lock().unwrap_or_else(PoisonError::into_inner);
            let deadline = target.as_ref().map(|target| target.deadline);
            assert_eq!(deadline, Some(Duration::from_secs(7)), "latest target wins");
        }
        for round in 1..=2 {
            assert!(std::thread::spawn(|| std::panic::panic_any("probe"))
                .join()
                .is_err());
            assert_eq!(probe.load(Ordering::SeqCst), round);
        }
        assert!(PREVIOUS.get().is_some());
    }
}
