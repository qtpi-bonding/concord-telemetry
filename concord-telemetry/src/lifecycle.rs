//! Service lifecycle events: started, stopping, alive, and the shutdown signal.
//!
//! The service name, version and instance id come from `init`, never from a
//! caller; `source_record_id` on every lifecycle event is the instance id (the
//! Resource's `service.instance.id`, a 16-digit lower-case hex string).

use crate::Event;
use std::sync::OnceLock;
use std::time::Instant;

/// Who this process is, fixed by `init` in every build.
pub(crate) struct Identity {
    pub service: String,
    pub version: String,
    pub instance_id: String,
    pub started: Instant,
}

static IDENTITY: OnceLock<Identity> = OnceLock::new();

pub(crate) fn set_identity(identity: Identity) {
    // `init` succeeds once per process, so this is the only writer.
    let _ = IDENTITY.set(identity);
}

fn identity() -> Result<&'static Identity, LifecycleError> {
    IDENTITY.get().ok_or(LifecycleError::NotInitialised)
}

fn uptime_ms(identity: &Identity) -> i64 {
    i64::try_from(identity.started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

#[derive(Clone, PartialEq, prost::Message, crate::Event)]
#[concord_event(package = "concord_telemetry.v1")]
pub struct ServiceStarted {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(string, tag = "2")]
    pub service: String,
    #[prost(string, tag = "3")]
    pub version: String,
}

#[derive(Clone, PartialEq, prost::Message, crate::Event)]
#[concord_event(package = "concord_telemetry.v1")]
pub struct ServiceStopping {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(string, tag = "2")]
    pub service: String,
    #[prost(string, optional, tag = "3")]
    pub reason: Option<String>,
    #[prost(int64, tag = "4")]
    pub uptime_ms: i64,
}

#[derive(Clone, PartialEq, prost::Message, crate::Event)]
#[concord_event(package = "concord_telemetry.v1")]
pub struct ServiceAlive {
    #[prost(string, tag = "1")]
    pub source_record_id: String,
    #[prost(string, tag = "2")]
    pub service: String,
    #[prost(int64, tag = "3")]
    pub uptime_ms: i64,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "concord.lifecycle")]
pub enum LifecycleError {
    #[error("telemetry not initialised")]
    #[coded(code = "not_initialised", class = Logical)]
    NotInitialised,
}

/// Emits `ServiceStarted`; the record's code location is the caller's.
#[track_caller]
pub fn started() -> Result<(), LifecycleError> {
    let identity = identity()?;
    let event = ServiceStarted {
        source_record_id: identity.instance_id.clone(),
        service: identity.service.clone(),
        version: identity.version.clone(),
    };
    emit_from_caller(&event);
    Ok(())
}

/// Emits `ServiceStopping` with an optional reason and the uptime since `init`.
#[track_caller]
pub fn stopping(reason: Option<&str>) -> Result<(), LifecycleError> {
    let identity = identity()?;
    let event = ServiceStopping {
        source_record_id: identity.instance_id.clone(),
        service: identity.service.clone(),
        reason: reason.map(str::to_owned),
        uptime_ms: uptime_ms(identity),
    };
    emit_from_caller(&event);
    Ok(())
}

#[track_caller]
fn emit_from_caller(event: &impl Event) {
    let at = std::panic::Location::caller();
    // `Location::file` is `'static`; the module is the library's own when the
    // caller's cannot be known, so only file and line are the caller's.
    crate::emit_at(at.file(), at.line(), module_path!(), || event.emit());
}

/// Stops the heartbeat when dropped.
#[cfg(feature = "lifecycle")]
pub struct Alive(tokio::task::JoinHandle<()>);

#[cfg(feature = "lifecycle")]
impl Drop for Alive {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Emits `ServiceAlive` every `period` (first one after one period) until the
/// returned handle is dropped. Needs a running tokio runtime.
#[cfg(feature = "lifecycle")]
pub fn spawn_alive(period: std::time::Duration) -> Result<Alive, LifecycleError> {
    let identity = identity()?;
    let handle = tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            ticks.tick().await;
            crate::event!(ServiceAlive {
                source_record_id: identity.instance_id.clone(),
                service: identity.service.clone(),
                uptime_ms: uptime_ms(identity),
            });
        }
    });
    Ok(Alive(handle))
}

/// Waits for ctrl-c or (unix) SIGTERM, emits `ServiceStopping` with reason
/// `ctrl_c` or `sigterm` exactly once, and returns. Both handlers are
/// registered before the first wait.
#[cfg(feature = "lifecycle")]
pub async fn shutdown_signal() {
    #[cfg(unix)]
    let reason = {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                result = tokio::signal::ctrl_c() => result.map(|()| "ctrl_c"),
                _ = term.recv() => Ok("sigterm"),
            },
            Err(error) => Err(error),
        }
    };
    #[cfg(not(unix))]
    let reason = tokio::signal::ctrl_c().await.map(|()| "ctrl_c");
    match reason {
        Ok(reason) => {
            let _ = stopping(Some(reason));
        }
        Err(error) => {
            tracing::warn!(%error, "waiting for a shutdown signal failed");
            std::future::pending::<()>().await;
        }
    }
}
