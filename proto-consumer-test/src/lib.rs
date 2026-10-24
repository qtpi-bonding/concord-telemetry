//! Generated from `proto/consumer/v1/events.proto`, which imports
//! `concord_telemetry/v1/principal.proto` through
//! `concord_telemetry_proto::proto_include_dir()`.
#![allow(clippy::all)] // generated code
include!(concat!(env!("OUT_DIR"), "/consumer.v1.rs"));
include!(concat!(env!("OUT_DIR"), "/consumer.v1.serde.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use concord_telemetry::{Principal, PrincipalKind, Verification};
    use prost::Message;

    #[test]
    fn principal_is_the_runtime_crates_type() {
        let ev = ThingHappened {
            source_record_id: "r1".into(),
            principal: Some(
                Principal::new(PrincipalKind::Agent, "k1")
                    .with_verified_by([Verification::ChallengeProof]),
            ),
        };
        assert_eq!(ThingHappened::decode(&ev.encode_to_vec()[..]).unwrap(), ev);
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            json["principal"]["verifiedBy"][0],
            "VERIFICATION_CHALLENGE_PROOF"
        );
        assert_eq!(serde_json::from_value::<ThingHappened>(json).unwrap(), ev);
    }
}
