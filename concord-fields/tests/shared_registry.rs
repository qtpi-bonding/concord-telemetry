use concord_fields::{label_names, parse, FieldType, Registry, SHARED};

fn shared() -> concord_fields::Parsed {
    parse(SHARED).expect("shared registry parses")
}

/// How many times `name` is declared as a key in the registry source.
fn declared(name: &str) -> usize {
    SHARED
        .lines()
        .filter(|line| {
            line.starts_with(&format!("{name} = ")) || line.starts_with(&format!("\"{name}\" = "))
        })
        .count()
}

#[test]
fn every_entry_has_a_type() {
    let parsed = shared();
    assert!(parsed.names.len() > 60, "{} entries", parsed.names.len());
    // Every entry line declares a type in the source, not only by default.
    for line in SHARED.lines().filter(|l| l.contains(" = { ")) {
        assert!(line.contains("type = \""), "{line}");
    }
}

#[test]
fn promoted_names_present_once() {
    let promoted: &[(&str, FieldType)] = &[
        ("outcome", FieldType::Str),
        ("ok", FieldType::Bool),
        ("count", FieldType::Int),
        ("reason", FieldType::Str),
        ("kind", FieldType::Str),
        ("command", FieldType::Str),
        ("listen", FieldType::Str),
        ("attempt", FieldType::Int),
        ("attempts", FieldType::Int),
        ("cause", FieldType::Str),
        ("status", FieldType::Str),
        ("node", FieldType::Str),
        ("nodes", FieldType::Int),
        ("principal", FieldType::Str),
        ("file", FieldType::Str),
        ("path", FieldType::Str),
        ("target", FieldType::Str),
        ("target_daemon_id", FieldType::Str),
        ("skipped", FieldType::Bool),
        ("skipped_count", FieldType::Int),
        ("cwd", FieldType::Str),
        ("pid", FieldType::Int),
        ("key_id", FieldType::Str),
        ("message_count", FieldType::Int),
        ("error_code", FieldType::Str),
        ("target_session_id", FieldType::Str),
        ("acp_session_id", FieldType::Str),
    ];
    let parsed = shared();
    for (name, ty) in promoted {
        assert_eq!(declared(name), 1, "{name}");
        assert_eq!(parsed.names[*name].ty, *ty, "{name}");
        assert!(!parsed.names[*name].label, "{name}");
    }
    for name in ["scope", "mode", "phase", "source"] {
        assert!(!parsed.names.contains_key(name), "{name} stays local");
    }
    assert!(!parsed.names.contains_key("gait_session_id"));
}

#[test]
fn only_command_is_free_text() {
    let parsed = shared();
    let free: Vec<_> = parsed
        .names
        .iter()
        .filter(|(_, def)| def.free_text)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(free, ["command"]);
}

#[test]
fn emitted_keys_are_registered() {
    let parsed = shared();
    for name in [
        "error.code",
        "error.class",
        "error.fingerprint",
        "error.source",
        "ingress",
    ] {
        assert_eq!(parsed.names[name].ty, FieldType::Str, "{name}");
        assert_eq!(declared(name), 1, "{name}");
    }
    assert!(parsed.names["error.type"].meaning.contains("semconv"));
    assert!(parsed.names["ingress"].meaning.contains("entry point"));
}

#[test]
fn lifecycle_fields_registered() {
    let parsed = shared();
    assert_eq!(parsed.names["service"].ty, FieldType::Str);
    assert_eq!(parsed.names["version"].ty, FieldType::Str);
    assert_eq!(parsed.names["uptime_ms"].ty, FieldType::Int);
}

#[test]
fn no_app_names_in_meanings() {
    let banned = [
        "grorg",
        "multimatrix",
        "structmux",
        "gait",
        "archegraph",
        "catuskoti",
        "concordium",
    ];
    for (name, def) in &shared().names {
        let meaning = def.meaning.to_lowercase();
        for word in banned {
            assert!(!meaning.contains(word), "{name}: {}", def.meaning);
        }
    }
}

#[test]
fn labels_unchanged() {
    assert_eq!(
        label_names(),
        [
            "action_id",
            "agent_id",
            "daemon_id",
            "delivery_id",
            "instance_id",
            "message_id",
            "place_id",
            "request_id",
            "session_id",
            "source_instance_id",
            "source_turn_id",
            "turn_id"
        ]
    );
    assert_eq!(Registry::shared().label_names().len(), 12);
}

#[test]
fn machine_names_are_gone_place_and_daemon_are_registered() {
    let parsed = shared();
    assert!(parsed.names["place_id"].label);
    assert_eq!(parsed.names["place_id"].ty, FieldType::Str);
    assert!(parsed.names["daemon_id"].label);
    assert_eq!(parsed.names["daemon_id"].ty, FieldType::Str);
    assert!(!parsed.names["target_daemon_id"].label);
    for name in ["machine_id", "offline_machine_id", "target_machine_id"] {
        assert!(!parsed.names.contains_key(name), "{name}");
    }
}

#[test]
fn the_shared_registry_maps_cause_kinds_to_labels() {
    let shared = shared();
    assert_eq!(
        shared.causes["instance"],
        concord_fields::CauseDef {
            label: "instance_id".to_string(),
            caller_label: Some("source_instance_id".to_string()),
        }
    );
    assert!(
        !shared
            .causes
            .values()
            .any(|cause| cause.label == "session_id"),
        "session_id is no cause label"
    );
    let labels = concord_fields::cause_labels(&shared);
    assert_eq!(labels.len(), 6, "{labels:?}");
    let found = labels
        .iter()
        .find(|(label, _)| label == "source_turn_id")
        .expect("source_turn_id is a cause label");
    assert_eq!(found.1, "turn");
}

#[test]
fn a_cause_label_must_be_a_registered_label() {
    let error = parse(
        "[fields]\nroom_id = { type = \"str\", meaning = \"x\" }\n[causes]\nx = { label = \"room_id\" }\n",
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            concord_fields::ParseError::CauseLabelNotALabel { .. }
        ),
        "{error:?}"
    );
    let error = parse("[causes]\nx = { label = \"not_a_field\" }\n").unwrap_err();
    assert!(
        matches!(
            error,
            concord_fields::ParseError::CauseLabelNotALabel { .. }
        ),
        "{error:?}"
    );
}

#[test]
fn a_cause_kind_is_a_lowercase_word() {
    for kind in ["Action", "a-b"] {
        let error =
            parse(&format!("[causes]\n{kind} = {{ label = \"action_id\" }}\n")).unwrap_err();
        assert!(
            matches!(error, concord_fields::ParseError::CauseKindName { .. }),
            "{kind}: {error:?}"
        );
    }
}

#[test]
fn a_label_serves_one_kind() {
    let error = parse("[fields]\naction_id = { type = \"str\", meaning = \"m\", label = true }\n[causes]\na = { label = \"action_id\" }\nb = { label = \"action_id\" }\n")
        .unwrap_err();
    assert!(
        matches!(error, concord_fields::ParseError::CauseLabelTwice { .. }),
        "{error:?}"
    );
    let error = parse("[fields]\naction_id = { type = \"str\", meaning = \"m\", label = true }\n[causes]\na = { label = \"action_id\", caller_label = \"action_id\" }\n")
        .unwrap_err();
    assert!(
        matches!(error, concord_fields::ParseError::CauseLabelTwice { .. }),
        "{error:?}"
    );
}
