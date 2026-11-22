use concord_fields::{label_names, parse, FieldType, FieldVerdict, ParseError, Registry};

#[test]
fn shared_registry_contains_seed_and_never_names() {
    let registry = Registry::shared();
    assert!(registry.names.contains_key("action_id"));
    assert!(!registry
        .names
        .keys()
        .any(|name| registry.never.contains(name)));
    assert_eq!(registry.check("action_id"), FieldVerdict::Ok);
    assert_eq!(registry.check("stdout"), FieldVerdict::Never);
    assert_eq!(registry.check("whatever"), FieldVerdict::Unregistered);
}

#[test]
fn local_registry_adds_names_and_rejects_conflicts() {
    let local = "[fields]\nnode_kind = { type = \"str\", meaning = \"kind\" }";
    let registry = Registry::shared()
        .with_local(local)
        .expect("overlay should parse");
    assert_eq!(registry.check("node_kind"), FieldVerdict::Ok);

    let never = "[fields]\nstdout = { type = \"str\", meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(never)
        .expect_err("never name must fail");
    assert!(error.to_string().contains("never"), "{error}");
}

#[test]
fn local_shadowing_a_shared_name_is_refused() {
    let shadow = "[fields]\naction_id = { type = \"str\", meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(shadow)
        .expect_err("shadow must fail");
    assert!(error.to_string().contains("shadows"), "{error}");
    assert!(error.to_string().contains("action_id"), "{error}");
}

#[test]
fn labels_are_the_fields_marked_label_true() {
    let registry = Registry::shared();
    assert!(registry.is_label("request_id"));
    assert!(!registry.is_label("room_id"));
    let local = "[fields]\nthread_id = { type = \"str\", meaning = \"x\", label = true }";
    let registry = registry.with_local(local).expect("overlay parses");
    assert!(registry.is_label("thread_id"));
}

#[test]
fn type_is_required() {
    let error = parse("[fields]\nname = { meaning = \"y\" }\n").unwrap_err();
    assert!(matches!(error, ParseError::MissingType { .. }), "{error:?}");
    assert!(error.to_string().contains("name"), "{error}");
}

#[test]
fn unknown_type_refused() {
    let error = parse("[fields]\nname = { type = \"string\", meaning = \"y\" }\n").unwrap_err();
    assert!(matches!(error, ParseError::UnknownType { .. }), "{error:?}");
    assert!(error.to_string().contains("string"), "{error}");
}

#[test]
fn every_type_name_is_read() {
    let parsed = parse(
        "[fields]\na = { type = \"str\", meaning = \"x\" }\nb = { type = \"int\", meaning = \"x\" }\nc = { type = \"float\", meaning = \"x\" }\nd = { type = \"bool\", meaning = \"x\" }\n",
    )
    .expect("parses");
    assert_eq!(parsed.names["a"].ty, FieldType::Str);
    assert_eq!(parsed.names["b"].ty, FieldType::Int);
    assert_eq!(parsed.names["c"].ty, FieldType::Float);
    assert_eq!(parsed.names["d"].ty, FieldType::Bool);
}

#[test]
fn label_must_be_str() {
    let error =
        parse("[fields]\nn = { type = \"int\", meaning = \"x\", label = true }\n").unwrap_err();
    assert!(matches!(error, ParseError::LabelNotStr { .. }), "{error:?}");
}

#[test]
fn free_text_and_label_exclusive() {
    let error = parse(
        "[fields]\nn = { type = \"str\", meaning = \"x\", label = true, free_text = true }\n",
    )
    .unwrap_err();
    assert!(
        matches!(error, ParseError::LabelAndFreeText { .. }),
        "{error:?}"
    );
}

#[test]
fn free_text_flag_read() {
    let registry = Registry::shared()
        .with_local("[fields]\nnote = { type = \"str\", meaning = \"x\", free_text = true }\nplain = { type = \"str\", meaning = \"x\" }\n")
        .expect("parses");
    assert!(registry.is_free_text("note"));
    assert!(!registry.is_free_text("plain"));
    assert!(!registry.is_free_text("unknown"));
    assert_eq!(concord_fields::FREE_TEXT_MAX, 1024);
}

#[test]
fn label_names_are_the_ten_labels_sorted() {
    let expected = [
        "action_id",
        "agent_id",
        "daemon_id",
        "delivery_id",
        "instance_id",
        "message_id",
        "place_id",
        "request_id",
        "session_id",
        "turn_id",
    ];
    assert_eq!(label_names(), expected);
    assert_eq!(Registry::shared().label_names(), expected);
}

#[test]
fn the_reserved_keys_check_ok() {
    let registry = Registry::shared();
    for name in [
        "event.type",
        "principal.kind",
        "principal.id",
        "principal.verified_by",
        "principal.via",
    ] {
        assert_eq!(registry.check(name), FieldVerdict::Ok, "{name}");
    }
    assert_eq!(
        registry.check("concord.label.x"),
        FieldVerdict::ReservedPrefix
    );
}
