use concord_lint::registry::{FieldVerdict, Registry};

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
    let local = "[fields]\nnode_kind = { meaning = \"kind\" }";
    let registry = Registry::shared()
        .with_local(local)
        .expect("overlay should parse");
    assert_eq!(registry.check("node_kind"), FieldVerdict::Ok);

    let shadow = "[fields]\naction_id = { meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(shadow)
        .expect_err("shadow must fail");
    assert!(error.contains("shadows"));

    let never = "[fields]\nstdout = { meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(never)
        .expect_err("never name must fail");
    assert!(error.contains("never"));
}

#[test]
fn labels_are_the_fields_marked_label_true() {
    let registry = Registry::shared();
    assert!(registry.is_label("request_id"));
    assert!(!registry.is_label("room_id"));
    let local = "[fields]\nthread_id = { meaning = \"x\", label = true }";
    let registry = registry.with_local(local).expect("overlay parses");
    assert!(registry.is_label("thread_id"));
}
