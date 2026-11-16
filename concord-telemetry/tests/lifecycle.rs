//! No `init` in this binary: the lifecycle calls refuse, emit nothing, and the
//! type names are the shared ones.
use concord_telemetry::lifecycle::{
    self, LifecycleError, ServiceAlive, ServiceStarted, ServiceStopping,
};
use concord_telemetry::Event;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

struct Count(Arc<AtomicUsize>);

impl<S: tracing::Subscriber> Layer<S> for Count {
    fn on_event(&self, _: &tracing::Event<'_>, _: Context<'_, S>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn event_type_names() {
    assert_eq!(
        ServiceStarted::TYPE_NAME,
        "concord_telemetry.v1.ServiceStarted"
    );
    assert_eq!(
        ServiceStopping::TYPE_NAME,
        "concord_telemetry.v1.ServiceStopping"
    );
    assert_eq!(ServiceAlive::TYPE_NAME, "concord_telemetry.v1.ServiceAlive");
}

#[test]
fn started_before_init_is_not_initialised() {
    let seen = Arc::new(AtomicUsize::new(0));
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(Count(Arc::clone(&seen))),
    )
    .expect("subscriber");
    assert!(matches!(
        lifecycle::started(),
        Err(LifecycleError::NotInitialised)
    ));
    assert!(matches!(
        lifecycle::stopping(Some("x")),
        Err(LifecycleError::NotInitialised)
    ));
    assert_eq!(seen.load(Ordering::SeqCst), 0, "nothing emitted");
    assert_eq!(
        coded_error::CodedError::code(&LifecycleError::NotInitialised),
        "concord.lifecycle.not_initialised"
    );
}

#[cfg(feature = "lifecycle")]
#[tokio::test]
async fn spawn_alive_before_init_is_not_initialised() {
    let result = lifecycle::spawn_alive(std::time::Duration::from_secs(1));
    assert!(matches!(result, Err(LifecycleError::NotInitialised)));
}
