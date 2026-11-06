use concord_telemetry::{error, error_anyhow};
use std::sync::{Arc, Mutex};
use tracing_subscriber::{layer::Context, layer::SubscriberExt, Layer};

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.action")]
enum ActionError {
    #[error("action a1 not found")]
    #[coded(code = "not_found", class = Logical)]
    NotFound,
}

#[derive(Clone, Debug)]
struct Recorded {
    name: &'static str,
    target: &'static str,
    level: tracing::Level,
    fields: Vec<(String, String)>,
}

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<Recorded>>>);

impl<S> Layer<S> for Capture
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct Visitor(Vec<(String, String)>);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push((field.name().to_owned(), format!("{value:?}")));
            }
        }

        let mut visitor = Visitor(Vec::new());
        event.record(&mut visitor);
        self.0.lock().unwrap().push(Recorded {
            name: event.metadata().name(),
            target: event.metadata().target(),
            level: *event.metadata().level(),
            fields: visitor.0,
        });
    }
}

fn capture() -> Arc<Mutex<Vec<Recorded>>> {
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(records.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let e = ActionError::NotFound;
        error!(&e, rpc.method = "GetAction");
        error!(&e, source = "x");
        let m = "GetAction";
        error!(&e, rpc.method = %m);
        error!(&e,);
        let anyhow_error = anyhow::anyhow!("boom");
        error_anyhow!(&anyhow_error, source = "test.source");
    });
    records
}

#[test]
fn records_facts_without_display_text() {
    let _lock = LOCK.lock().unwrap();
    let records = capture();
    let records = records.lock().unwrap();
    let coded = &records[0];
    assert_eq!(coded.name, "error");
    assert_eq!(coded.target, concord_telemetry::EVENT_TARGET);
    assert_eq!(coded.level, tracing::Level::ERROR);

    let field = |name: &str| {
        coded
            .fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
            .unwrap()
    };
    assert_eq!(field("error.code"), "\"test.action.not_found\"");
    assert_eq!(field("error.class"), "\"logical\"");
    assert_eq!(
        field("error.fingerprint"),
        format!(
            "\"{}\"",
            coded_error::fingerprint(
                "test.action.not_found",
                coded_error::ErrorClass::Logical,
                module_path!()
            )
        )
    );
    assert!(coded
        .fields
        .iter()
        .any(|(name, value)| { name == "rpc.method" && value.contains("GetAction") }));
    assert!(records.iter().all(|record| {
        record
            .fields
            .iter()
            .filter(|(name, _)| name != "error.chain" && name != "error.message")
            .all(|(_, value)| !value.contains("a1 not found"))
    }));

    let anyhow_record = records.last().unwrap();
    assert!(anyhow_record
        .fields
        .iter()
        .any(|(name, value)| name == "error.code" && value == "\"unknown\""));
    assert!(anyhow_record
        .fields
        .iter()
        .any(|(name, value)| name == "error.source" && value == "\"test.source\""));
    assert!(records.iter().all(|record| {
        record
            .fields
            .iter()
            .filter(|(name, _)| name != "error.chain" && name != "error.message")
            .all(|(_, value)| !value.contains("boom"))
    }));
}

#[test]
fn error_anyhow_without_source_uses_the_module_path() {
    let _lock = LOCK.lock().unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(records.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let coded = coded_error::into_anyhow(ActionError::NotFound);
        error_anyhow!(&coded, rpc.method = "GetAction");
        error_anyhow!(&anyhow::anyhow!("boom"));
    });
    let records = records.lock().unwrap();
    let get =
        |r: &Recorded, name: &str| r.fields.iter().find(|(f, _)| f == name).unwrap().1.clone();
    assert_eq!(
        get(&records[0], "error.source"),
        format!("\"{}\"", module_path!())
    );
    assert_eq!(get(&records[0], "rpc.method"), "\"GetAction\"");
    assert_eq!(
        get(&records[0], "error.fingerprint"),
        format!(
            "\"{}\"",
            coded_error::fingerprint(
                "test.action.not_found",
                coded_error::ErrorClass::Logical,
                module_path!()
            )
        )
    );
    assert_eq!(get(&records[1], "error.code"), "\"unknown\"");
    assert_eq!(
        get(&records[1], "error.source"),
        format!("\"{}\"", module_path!())
    );
}

#[test]
fn error_record_has_chain_with_text_under_full() {
    let _lock = LOCK.lock().unwrap();
    concord_telemetry::set_error_text(concord_telemetry::ErrorText::Full);
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(records.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let error = ActionError::NotFound;
        error!(&error);
    });
    let records = records.lock().unwrap();
    let chain = records[0]
        .fields
        .iter()
        .find(|(name, _)| name == "error.chain")
        .unwrap()
        .1
        .clone();
    assert!(chain.contains("test.action.not_found"));
    assert!(chain.contains("action a1 not found"));
    assert!(records[0]
        .fields
        .iter()
        .any(|(name, value)| name == "error.message" && value.contains("action a1 not found")));
}

#[test]
fn own_mode_omits_external_text_but_keeps_codes() {
    let _lock = LOCK.lock().unwrap();
    concord_telemetry::set_error_text(concord_telemetry::ErrorText::Own);
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(records.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let error = anyhow::anyhow!("external detail");
        error_anyhow!(&error);
    });
    let records = records.lock().unwrap();
    let chain = records[0]
        .fields
        .iter()
        .find(|(name, _)| name == "error.chain")
        .unwrap()
        .1
        .clone();
    assert!(chain.contains("external"));
    assert!(!chain.contains("external detail"));
}
