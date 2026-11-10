//! How much work the baggage layer does. The end-of-span collapse hides
//! repeated writes, so these count them.

use concord_telemetry::context;
use concord_telemetry::testing::{capture_exclusive, span_attr, spans, stamp_count};
use std::sync::{Arc, Barrier};
use tracing::info_span;

fn labelled_parent(name: &'static str) -> tracing::Span {
    let parent = info_span!(parent: None, "stamp-parent", test = name);
    parent.in_scope(|| {
        context::add("action_id", "A");
        context::add("turn_id", "T");
    });
    parent
}

#[test]
fn entering_a_stamped_span_again_does_no_stamping_work() {
    let _c = capture_exclusive();
    let parent = labelled_parent("again");
    let child = parent.in_scope(|| info_span!("stamp-again"));
    let before = stamp_count();
    child.in_scope(|| {});
    assert_eq!(
        stamp_count() - before,
        2,
        "the first entry stamps both labels"
    );
    let after_first = stamp_count();
    for _ in 0..10 {
        child.in_scope(|| {});
    }
    assert_eq!(stamp_count(), after_first, "later entries stamp nothing");
    drop(child);
    drop(parent);
    let exported = spans()
        .into_iter()
        .find(|s| s.name == "stamp-again")
        .expect("child exported");
    assert_eq!(span_attr(&exported, "action_id").as_deref(), Some("A"));
}

#[test]
fn the_same_span_entered_on_two_threads_stamps_each_label_once() {
    let _c = capture_exclusive();
    for _ in 0..50 {
        let parent = labelled_parent("threads");
        let child = parent.in_scope(|| info_span!("stamp-threads"));
        let before = stamp_count();
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let child = child.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    child.in_scope(|| {});
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread");
        }
        assert_eq!(stamp_count() - before, 2);
    }
}

#[test]
fn a_record_racing_the_first_enter() {
    let _c = capture_exclusive();
    for _ in 0..100 {
        let parent = labelled_parent("record-race");
        let child =
            parent.in_scope(|| info_span!("race-record", action_id = tracing::field::Empty));
        let barrier = Arc::new(Barrier::new(2));
        let (b1, c1) = (Arc::clone(&barrier), child.clone());
        let (b2, c2) = (barrier, child.clone());
        let enter = std::thread::spawn(move || {
            b1.wait();
            c1.in_scope(|| {});
        });
        let record = std::thread::spawn(move || {
            b2.wait();
            c2.record("action_id", "A");
        });
        enter.join().expect("enter");
        record.join().expect("record");
        drop(child);
    }
    let exported: Vec<_> = spans()
        .into_iter()
        .filter(|s| s.name == "race-record")
        .collect();
    assert_eq!(exported.len(), 100);
    for span in &exported {
        assert_eq!(span_attr(span, "action_id").as_deref(), Some("A"));
        assert_eq!(span_attr(span, "concord.label_conflicts"), None);
    }
}

#[test]
fn context_add_racing_the_first_enter() {
    let _c = capture_exclusive();
    for _ in 0..100 {
        let parent = labelled_parent("add-race");
        let child = parent.in_scope(|| info_span!("race-add"));
        let barrier = Arc::new(Barrier::new(2));
        let (b1, c1) = (Arc::clone(&barrier), child.clone());
        let (b2, c2) = (barrier, child.clone());
        let enter = std::thread::spawn(move || {
            b1.wait();
            c1.in_scope(|| {});
        });
        let add = std::thread::spawn(move || {
            b2.wait();
            c2.in_scope(|| context::add("turn_id", "NEW"));
        });
        enter.join().expect("enter");
        add.join().expect("add");
        child.in_scope(|| info_span!("race-add-grandchild").in_scope(|| {}));
        drop(child);
    }
    let all = spans();
    let children: Vec<_> = all.iter().filter(|s| s.name == "race-add").collect();
    let grandchildren: Vec<_> = all
        .iter()
        .filter(|s| s.name == "race-add-grandchild")
        .collect();
    assert_eq!(children.len(), 100);
    assert_eq!(grandchildren.len(), 100);
    for span in children.iter().chain(grandchildren.iter()) {
        assert_eq!(
            span_attr(span, "turn_id").as_deref(),
            Some("NEW"),
            "{}",
            span.name
        );
        assert_eq!(span_attr(span, "action_id").as_deref(), Some("A"));
    }
}
