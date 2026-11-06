use concord_telemetry::{error, error_text, set_error_text, ErrorText};
use std::sync::{Arc, Barrier, Mutex};

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.text")]
enum TextError {
    #[error("text error")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

#[test]
fn error_marks_current_span_status_error() {
    let _lock = LOCK.lock().unwrap();
    let capture = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::testing::set_otel_active(true);
    let span = tracing::info_span!("error-span");
    {
        let _entered = span.enter();
        error!(&TextError::Failed);
    }
    drop(span);
    let expected_status = opentelemetry::trace::Status::Error {
        description: "error".into(),
    };
    assert!(concord_telemetry::testing::spans().iter().any(|span| {
        span.status == expected_status
            && span.attributes.iter().any(|attribute| {
                attribute.key.as_str() == "error.code"
                    && attribute.value.to_string().contains("test.text.failed")
            })
    }));
    drop(capture);
}

#[test]
fn concord_error_text_env_sets_initial_mode() {
    let _lock = LOCK.lock().unwrap();
    let _capture = concord_telemetry::testing::capture_exclusive();
    std::env::set_var("CONCORD_ERROR_TEXT", "own");
    // init reads the variable before installing its subscriber.
    let _ = concord_telemetry::init(concord_telemetry::Config::new("test", "0"));
    assert_eq!(error_text(), ErrorText::Own);
    std::env::remove_var("CONCORD_ERROR_TEXT");
    set_error_text(ErrorText::Full);
}

#[test]
fn flipping_error_text_concurrently_is_consistent() {
    let _lock = LOCK.lock().unwrap();
    let capture = concord_telemetry::testing::capture_exclusive();
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..1000 {
                error!(&TextError::Failed);
            }
        }));
    }
    barrier.wait();
    for i in 0..1000 {
        set_error_text(if i % 2 == 0 {
            ErrorText::Own
        } else {
            ErrorText::Full
        });
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let records = capture.errors_where(|_| true);
    assert_eq!(records.len(), 4000);
    for record in records {
        let chain = format!("{:?}", record.fields.get("error.chain").expect("chain"));
        assert!(chain.contains("text error") || !chain.contains("message"));
    }
}
