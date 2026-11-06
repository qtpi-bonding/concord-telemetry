use concord_telemetry::{error, set_error_text, ErrorText};
use std::io;
use std::sync::{Arc, Barrier, Mutex};

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.text")]
enum TextError {
    #[error("text error")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.mixed")]
enum MixedError {
    #[error("mixed own error")]
    #[coded(code = "failed", class = Logical)]
    Failed {
        #[source]
        source: io::Error,
    },
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
                let error = anyhow::Error::new(MixedError::Failed {
                    source: io::Error::other("external detail"),
                });
                concord_telemetry::error_anyhow!(&error);
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
        let encoded = match record.fields.get("error.chain").expect("chain") {
            concord_telemetry::AttrValue::Str(value) => value,
            _ => panic!("chain was not text"),
        };
        let chain: serde_json::Value = serde_json::from_str(encoded).expect("valid chain JSON");
        let external = chain
            .as_array()
            .expect("chain array")
            .iter()
            .filter(|link| link["origin"] == "external")
            .collect::<Vec<_>>();
        assert!(!external.is_empty());
        let has_external_text = external[0].get("message").is_some();
        assert!(external
            .iter()
            .all(|link| link.get("message").is_some() == has_external_text));
    }
}
