//! Own binary: `grpc::reject*` map an error to a `tonic::Status` and record it
//! at the caller's location.
#![cfg(all(feature = "grpc-status", feature = "testing"))]

use concord_telemetry::grpc::{reject, reject_anyhow, reject_anyhow_redacted};
use concord_telemetry::testing::capture_exclusive;
use concord_telemetry::{AttrValue, Attrs};
use tonic::Code;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.grpc_status")]
enum RpcError {
    #[error("bad input")]
    #[coded(code = "bad_input", class = Logical)]
    BadInput,
    #[error("db down at secret-host")]
    #[coded(code = "db_down", class = Infrastructure)]
    DbDown,
    #[error("try again")]
    #[coded(code = "busy", class = Transient)]
    Busy,
}

fn str_attr<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    match attrs.get(key) {
        Some(AttrValue::Str(text)) => Some(text),
        _ => None,
    }
}

#[test]
fn reject_maps_code_and_emits_record() {
    let captured = capture_exclusive();
    let cases = [
        (
            RpcError::BadInput,
            Code::InvalidArgument,
            "bad_input",
            "logical",
        ),
        (
            RpcError::DbDown,
            Code::Internal,
            "db_down",
            "infrastructure",
        ),
        (RpcError::Busy, Code::Unavailable, "busy", "transient"),
    ];
    for (error, code, name, class) in cases {
        let method = "/pkg.Service/Map";
        let status = reject(method, error);
        assert_eq!(status.code(), code, "{name}");
        let full = format!("test.grpc_status.{name}");
        let found = captured.errors_where(|record| record.code == full);
        assert_eq!(found.len(), 1, "{name}");
        assert_eq!(found[0].class, class);
        assert_eq!(str_attr(&found[0].fields, "rpc.method"), Some(method));
    }
}

#[test]
fn record_location_is_the_caller() {
    let captured = capture_exclusive();
    let line = line!() + 1;
    let _status = reject("/pkg.Service/Where", RpcError::Busy);
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    let path = str_attr(&errors[0], "code.file.path").expect("code.file.path");
    assert!(path.ends_with("tests/grpc_status.rs"), "{path}");
    assert_eq!(
        errors[0].get("code.line.number"),
        Some(&AttrValue::I64(i64::from(line)))
    );

    let line = line!() + 1;
    let _status = reject_anyhow(
        "/pkg.Service/Where",
        &coded_error::into_anyhow(RpcError::Busy),
    );
    let errors = captured.attrs_for("error");
    let last = errors.last().expect("anyhow record");
    assert_eq!(
        last.get("code.line.number"),
        Some(&AttrValue::I64(i64::from(line)))
    );
    assert!(str_attr(last, "code.file.path")
        .expect("code.file.path")
        .ends_with("tests/grpc_status.rs"));

    let line = line!() + 1;
    let _status = reject_anyhow_redacted(
        "/pkg.Service/Where",
        &coded_error::into_anyhow(RpcError::Busy),
        "internal error",
    );
    let errors = captured.attrs_for("error");
    let last = errors.last().expect("redacted record");
    assert_eq!(
        last.get("code.line.number"),
        Some(&AttrValue::I64(i64::from(line)))
    );
}

#[test]
fn redacted_hides_message() {
    let captured = capture_exclusive();
    let error =
        coded_error::into_anyhow(RpcError::DbDown).context("connecting to secret-host:5432");
    let status = reject_anyhow_redacted("/pkg.Service/Hide", &error, "internal error");
    assert_eq!(status.code(), Code::Internal);
    assert_eq!(status.message(), "internal error");
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        str_attr(&errors[0], "error.code"),
        Some("test.grpc_status.db_down")
    );
    let chain = str_attr(&errors[0], "error.chain").expect("error.chain");
    assert!(chain.contains("test.grpc_status.db_down"), "{chain}");
    assert_eq!(
        str_attr(&errors[0], "rpc.method"),
        Some("/pkg.Service/Hide")
    );
}

#[test]
fn anyhow_downcasts_coded_cause() {
    let captured = capture_exclusive();
    let error = coded_error::into_anyhow(RpcError::BadInput).context("handling the request");
    let status = reject_anyhow("/pkg.Service/Cause", &error);
    assert_eq!(status.code(), Code::InvalidArgument);
    let found = captured.errors_where(|record| record.code == "test.grpc_status.bad_input");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].class, "logical");
    assert_eq!(
        str_attr(&found[0].fields, "rpc.method"),
        Some("/pkg.Service/Cause")
    );

    let plain = anyhow::anyhow!("not coded");
    let status = reject_anyhow("/pkg.Service/Cause", &plain);
    assert_eq!(status.code(), Code::Unknown);
}
