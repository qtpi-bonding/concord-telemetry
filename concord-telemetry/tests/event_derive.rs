#[path = "proto/fixtures.rs"]
mod fixtures;

use concord_telemetry::{
    AttrValue, Attrs, Event, Principal, PrincipalKind, RebuildError, Verification,
};
use fixtures::{ActionStatusChanged, IdentityStamped, PbjsonStamped, Scalars, Status};
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Attrs>>);

struct Visitor(Attrs);
impl tracing::field::Visit for Visitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .insert(field.name().to_string(), AttrValue::Str(value.to_string()));
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0
            .insert(field.name().to_string(), AttrValue::I64(value));
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0
            .insert(field.name().to_string(), AttrValue::U64(value));
    }
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.0
            .insert(field.name().to_string(), AttrValue::F64(value));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0
            .insert(field.name().to_string(), AttrValue::Bool(value));
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(
            field.name().to_string(),
            AttrValue::Str(format!("{value:?}")),
        );
    }
}
impl<S> Layer<S> for Recorder
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut visitor = Visitor(BTreeMap::new());
        event.record(&mut visitor);
        let mut attrs = self.0.lock().unwrap();
        *attrs = visitor.0;
        assert_eq!(event.metadata().target(), concord_telemetry::EVENT_TARGET);
        assert_eq!(event.metadata().level(), &tracing::Level::INFO);
        assert!(!event.metadata().name().is_empty());
    }
}
use std::collections::BTreeMap;

fn recorded<E: Event>(event: &E) -> Attrs {
    let recorder = Recorder::default();
    let attrs = recorder.0.clone();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(recorder), || {
        event.emit()
    });
    let result = attrs.lock().unwrap().clone();
    assert_eq!(
        result.get("event.type"),
        Some(&AttrValue::Str(E::TYPE_NAME.to_string()))
    );
    result
}

fn principal() -> Principal {
    Principal::new(PrincipalKind::Agent, "agent-7")
        .with_verified_by([
            Verification::DeliveryAttestation,
            Verification::DelegationCert,
        ])
        .with_via("delegation")
}

fn action(at: Option<prost_types::Timestamp>) -> ActionStatusChanged {
    ActionStatusChanged {
        source_record_id: "record-1".into(),
        action_id: "action-1".into(),
        old_status: Status::Ready as i32,
        new_status: Status::Running as i32,
        principal: Some(principal()),
        at,
        took: Some(prost_types::Duration {
            seconds: 2,
            nanos: 3,
        }),
        attempt: -4,
        retried: true,
        labels: vec!["one".into(), "two".into()],
        note: None,
    }
}

#[test]
fn type_name_and_all_action_attributes_are_rendered() {
    assert_eq!(
        ActionStatusChanged::TYPE_NAME,
        "test.events.v1.ActionStatusChanged"
    );
    let attrs = recorded(&action(Some(prost_types::Timestamp {
        seconds: 1792843200,
        nanos: 500_000_000,
    })));
    assert_eq!(
        attrs.get("source_record_id"),
        Some(&AttrValue::Str("record-1".into()))
    );
    assert_eq!(
        attrs.get("action_id"),
        Some(&AttrValue::Str("action-1".into()))
    );
    assert_eq!(
        attrs.get("old_status"),
        Some(&AttrValue::Str("READY".into()))
    );
    assert_eq!(
        attrs.get("new_status"),
        Some(&AttrValue::Str("RUNNING".into()))
    );
    assert_eq!(
        attrs.get("principal.kind"),
        Some(&AttrValue::Str("PRINCIPAL_KIND_AGENT".into()))
    );
    assert_eq!(
        attrs.get("principal.id"),
        Some(&AttrValue::Str("agent-7".into()))
    );
    assert_eq!(
        attrs.get("principal.verified_by"),
        Some(&AttrValue::Str(
            "[\"VERIFICATION_DELIVERY_ATTESTATION\",\"VERIFICATION_DELEGATION_CERT\"]".into()
        ))
    );
    assert_eq!(
        attrs.get("principal.via"),
        Some(&AttrValue::Str("delegation".into()))
    );
    assert_eq!(
        attrs.get("at"),
        Some(&AttrValue::Str("2026-10-24T12:00:00.500Z".into()))
    );
    assert_eq!(
        attrs.get("took"),
        Some(&AttrValue::Str("2.000000003s".into()))
    );
    assert_eq!(attrs.get("attempt"), Some(&AttrValue::I64(-4)));
    assert_eq!(attrs.get("retried"), Some(&AttrValue::Bool(true)));
    assert_eq!(
        attrs.get("labels"),
        Some(&AttrValue::Str("[\"one\",\"two\"]".into()))
    );
    assert!(!attrs.contains_key("note"));
}

#[test]
fn round_trips_timestamp_sources_and_zero_nanos() {
    let original = action(Some(prost_types::Timestamp {
        seconds: 1792843200,
        nanos: 0,
    }));
    assert_eq!(
        ActionStatusChanged::from_attributes(&recorded(&original)).unwrap(),
        original
    );
    let pb = PbjsonStamped {
        source_record_id: "pb".into(),
        at: Some(pbjson_types::Timestamp {
            seconds: 3,
            nanos: 4,
        }),
    };
    assert_eq!(PbjsonStamped::from_attributes(&recorded(&pb)).unwrap(), pb);
    let identity = IdentityStamped {
        source_record_id: "id".into(),
        at: Some(fixtures::identity_crypto::Timestamp {
            seconds: 8,
            nanos: 9,
        }),
    };
    assert_eq!(
        IdentityStamped::from_attributes(&recorded(&identity)).unwrap(),
        identity
    );
}

#[test]
fn scalar_rules_and_unrecognised_enum_are_lossless_or_error() {
    let value = Scalars {
        source_record_id: "s".into(),
        big: u64::MAX,
        count: u32::MAX,
        signed: i64::MAX,
        ratio: 1.25,
        precise: -2.5,
        enabled: true,
        status: Some(Status::Running as i32),
    };
    let attrs = recorded(&value);
    assert_eq!(
        attrs.get("big"),
        Some(&AttrValue::Str(u64::MAX.to_string()))
    );
    assert_eq!(attrs.get("count"), Some(&AttrValue::I64(u32::MAX as i64)));
    assert_eq!(attrs.get("signed"), Some(&AttrValue::I64(i64::MAX)));
    assert_eq!(Scalars::from_attributes(&attrs).unwrap(), value);
    let mut extremes = attrs.clone();
    extremes.insert("signed".into(), AttrValue::I64(i64::MIN));
    let rebuilt = Scalars::from_attributes(&extremes).unwrap();
    assert_eq!(rebuilt.signed, i64::MIN);
    extremes.insert("ratio".into(), AttrValue::I64(3));
    extremes.insert("precise".into(), AttrValue::F64(4.5));
    let rebuilt = Scalars::from_attributes(&extremes).unwrap();
    assert_eq!(rebuilt.ratio, 3.0);
    assert_eq!(rebuilt.precise, 4.5);
    let mut bad = attrs;
    bad.insert("status".into(), AttrValue::Str("UNRECOGNIZED".into()));
    assert!(matches!(
        Scalars::from_attributes(&bad),
        Err(RebuildError::Parse(_))
    ));
}

#[test]
fn optional_values_are_absent_and_event_filter_target_is_stable() {
    let mut value = action(None);
    value.principal.as_mut().unwrap().via = None;
    let attrs = recorded(&value);
    assert!(!attrs.contains_key("at"));
    assert!(!attrs.contains_key("principal.via"));
    let filter = tracing_subscriber::filter::Targets::new()
        .with_target(concord_telemetry::EVENT_TARGET, tracing::Level::INFO);
    let recorder = Recorder::default();
    let attrs = recorder.0.clone();
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(filter).with(recorder),
        || value.emit(),
    );
    assert!(attrs.lock().unwrap().contains_key("event.type"));
}
