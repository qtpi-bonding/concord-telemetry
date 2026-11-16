use concord_telemetry::init_failure_line;
use std::fmt;

#[derive(Debug)]
struct Link {
    text: String,
    next: Option<Box<Link>>,
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Link {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.next
            .as_deref()
            .map(|n| n as &(dyn std::error::Error + 'static))
    }
}

fn link(text: &str, next: Option<Link>) -> Link {
    Link {
        text: text.to_owned(),
        next: next.map(Box::new),
    }
}

#[test]
fn line_shape() {
    let line = init_failure_line("svc", &link("boom", None));
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 4);
    assert_eq!(object["level"], "error");
    assert_eq!(object["code"], "telemetry_init_failed");
    assert_eq!(object["service"], "svc");
    assert_eq!(object["message"], "boom");
    assert!(line.starts_with(
        r#"{"level":"error","code":"telemetry_init_failed","service":"svc","message":"#
    ));
}

#[test]
fn source_chain_joined() {
    let err = link("a", Some(link("b", Some(link("c", None)))));
    let line = init_failure_line("svc", &err);
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["message"], "a: b: c");
}

#[test]
fn escapes_quotes_and_newlines() {
    let line = init_failure_line("svc", &link("x\"y\nz", None));
    assert!(!line.contains('\n'));
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["message"], "x\"y\nz");
}

#[test]
fn no_trailing_newline_in_line() {
    let line = init_failure_line("svc", &link("boom\n", None));
    assert!(!line.ends_with('\n'));
    assert!(!line.contains('\n'));
}
