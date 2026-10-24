use concord_telemetry::{Principal, PrincipalKind, Verification};
use prost::Message;

#[test]
fn builder_and_attrs() {
    let p = Principal::new(PrincipalKind::Agent, "p256:04ab")
        .with_verified_by([
            Verification::DeliveryAttestation,
            Verification::DelegationCert,
        ])
        .with_via("instance-7");
    assert_eq!(p.id, "p256:04ab");
    assert_eq!(p.kind().as_str_name(), "PRINCIPAL_KIND_AGENT");
    assert_eq!(
        p.verified_by_attr(),
        r#"["VERIFICATION_DELIVERY_ATTESTATION","VERIFICATION_DELEGATION_CERT"]"#
    );
    assert_eq!(p.via.as_deref(), Some("instance-7"));
    let asserted = Principal::new(PrincipalKind::Daemon, "x");
    assert_eq!(asserted.verified_by_attr(), "[]");
}

#[test]
fn proto_and_json_round_trip() {
    let p =
        Principal::new(PrincipalKind::Human, "u1").with_verified_by([Verification::GrpcCallerAuth]);
    assert_eq!(Principal::decode(&p.encode_to_vec()[..]).unwrap(), p);
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"kind": "PRINCIPAL_KIND_HUMAN", "id": "u1", "verifiedBy": ["VERIFICATION_GRPC_CALLER_AUTH"]})
    );
    assert_eq!(serde_json::from_value::<Principal>(v).unwrap(), p);
}

// The Principal extern_path claim: a hand-written pbjson-style struct holding Option<Principal> serializes through serde.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Holder {
    #[serde(skip_serializing_if = "Option::is_none")]
    who: Option<Principal>,
}

#[test]
fn principal_inside_a_serde_struct() {
    let h = Holder {
        who: Some(Principal::new(PrincipalKind::Agent, "a").with_via("i")),
    };
    assert_eq!(
        serde_json::from_str::<Holder>(&serde_json::to_string(&h).unwrap()).unwrap(),
        h
    );
}
