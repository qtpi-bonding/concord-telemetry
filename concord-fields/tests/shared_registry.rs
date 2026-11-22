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
            "turn_id"
        ]
    );
    assert_eq!(Registry::shared().label_names().len(), 10);
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
