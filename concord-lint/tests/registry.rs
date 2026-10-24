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
    let local = "[fields]\nnode_kind = { type = \"str\", meaning = \"kind\" }";
    let registry = Registry::shared()
        .with_local(local)
        .expect("overlay should parse");
    assert_eq!(registry.check("node_kind"), FieldVerdict::Ok);

    let shadow = "[fields]\naction_id = { type = \"str\", meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(shadow)
        .expect_err("shadow must fail");
    assert!(error.contains("shadows"));

    let never = "[fields]\nstdout = { type = \"str\", meaning = \"bad\" }";
    let error = Registry::shared()
        .with_local(never)
        .expect_err("never name must fail");
    assert!(error.contains("never"));
}
