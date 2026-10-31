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

#[test]
fn challenge_and_root_signature_verifications() {
    assert_eq!(Verification::ChallengeProof as i32, 6);
    assert_eq!(Verification::RootSignature as i32, 7);
    assert_eq!(
        Verification::from_str_name("VERIFICATION_CHALLENGE_PROOF"),
        Some(Verification::ChallengeProof)
    );
    assert_eq!(
        Verification::from_str_name("VERIFICATION_ROOT_SIGNATURE"),
        Some(Verification::RootSignature)
    );
    let p = Principal::new(PrincipalKind::Agent, "k")
        .with_verified_by([Verification::ChallengeProof, Verification::RootSignature]);
    assert_eq!(
        p.verified_by_attr(),
        r#"["VERIFICATION_CHALLENGE_PROOF","VERIFICATION_ROOT_SIGNATURE"]"#
    );
    assert_eq!(Principal::decode(&p.encode_to_vec()[..]).unwrap(), p);
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(serde_json::from_value::<Principal>(v).unwrap(), p);
}

// The hand-written Rust enum must stay in step with the shipped .proto.
#[test]
fn rust_verification_matches_proto_file() {
    let proto =
        include_str!("../../concord-telemetry-proto/proto/concord_telemetry/v1/principal.proto");
    let body = proto
        .split("enum Verification {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    let mut n = 0;
    for line in body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("VERIFICATION_"))
    {
        let (name, num) = line.trim_end_matches(';').split_once(" = ").unwrap();
        let v =
            Verification::from_str_name(name).unwrap_or_else(|| panic!("{name} missing in Rust"));
        assert_eq!(v as i32, num.parse::<i32>().unwrap(), "{name}");
        n += 1;
    }
    assert_eq!(n, 1 + Verification::RootSignature as usize);
}

#[test]
fn service_kind_round_trips() {
    let p = Principal::new(PrincipalKind::Service, "central");
    assert_eq!(PrincipalKind::Service as i32, 4);
    assert_eq!(p.kind().as_str_name(), "PRINCIPAL_KIND_SERVICE");
    assert_eq!(
        PrincipalKind::from_str_name("PRINCIPAL_KIND_SERVICE"),
        Some(PrincipalKind::Service)
    );
    assert_eq!(Principal::decode(&p.encode_to_vec()[..]).unwrap(), p);
}

#[test]
fn rust_principal_kind_matches_proto_file() {
    let proto =
        include_str!("../../concord-telemetry-proto/proto/concord_telemetry/v1/principal.proto");
    let body = proto
        .split("enum PrincipalKind {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    let mut n = 0;
    for line in body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("PRINCIPAL_KIND_"))
    {
        let (name, num) = line.trim_end_matches(';').split_once(" = ").unwrap();
        let v =
            PrincipalKind::from_str_name(name).unwrap_or_else(|| panic!("{name} missing in Rust"));
        assert_eq!(v as i32, num.parse::<i32>().unwrap(), "{name}");
        n += 1;
    }
    assert_eq!(n, 1 + PrincipalKind::Service as usize);
}
