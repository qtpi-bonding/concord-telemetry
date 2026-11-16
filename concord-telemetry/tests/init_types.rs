use concord_telemetry::{Config, InitError};

#[test]
fn config_is_clone_debug_eq() {
    let c = Config::new("s", "1");
    assert_eq!(c.clone(), c);
    assert!(format!("{c:?}").contains("service_name"));
}

#[test]
fn exporter_error_keeps_its_source() {
    let io_err = std::io::Error::new(std::io::ErrorKind::Other, "boom");
    let e = InitError::Exporter(Box::new(io_err));
    assert!(std::error::Error::source(&e).is_some());
}
