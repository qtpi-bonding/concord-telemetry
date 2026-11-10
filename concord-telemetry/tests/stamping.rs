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

struct ClearHook;
impl Drop for ClearHook {
    fn drop(&mut self) {
        concord_telemetry::testing::set_enter_hook(None);
    }
}

/// Parks the first thread that reaches the hook until `release` is sent.
fn park_first_enter() -> (
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
    ClearHook,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Mutex};
    let (parked_tx, parked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    concord_telemetry::testing::set_enter_hook(Some(Box::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            parked_tx.send(()).expect("parked");
            let _ = release_rx.lock().expect("release").recv();
        }
    })));
    (parked_rx, release_tx, ClearHook)
}

#[test]
fn a_context_add_does_not_wait_on_a_parked_first_enter_and_the_latest_value_exports() {
    use std::sync::mpsc;
    use std::time::Duration;
    let _c = capture_exclusive();
    let parent = labelled_parent("parked");
    let child = parent.in_scope(|| info_span!("stamp-parked"));
    let (parked, release, _clear) = park_first_enter();

    let first = child.clone();
    let t1 = std::thread::spawn(move || first.in_scope(|| {}));
    parked
        .recv_timeout(Duration::from_secs(10))
        .expect("T1 parks in on_enter");

    let (done_tx, done_rx) = mpsc::channel();
    let second = child.clone();
    let t2 = std::thread::spawn(move || {
        second.in_scope(|| context::add("turn_id", "NEW"));
        done_tx.send(()).expect("done");
    });
    let finished = done_rx.recv_timeout(Duration::from_secs(10));
    release.send(()).expect("release T1");
    finished.expect("add finished while T1 was parked in on_enter");
    t1.join().expect("T1");
    t2.join().expect("T2");

    drop(child);
    let exported = spans()
        .into_iter()
        .find(|s| s.name == "stamp-parked")
        .expect("exported");
    assert_eq!(span_attr(&exported, "turn_id").as_deref(), Some("NEW"));
    assert_eq!(span_attr(&exported, "action_id").as_deref(), Some("A"));
}

#[test]
fn a_context_add_on_a_span_with_no_labels_marks_it_stamped() {
    let _c = capture_exclusive();
    let span = info_span!(parent: None, "stamp-fresh");
    span.in_scope(|| context::add("action_id", "A"));
    let before = stamp_count();
    span.in_scope(|| {});
    span.in_scope(|| {});
    assert_eq!(stamp_count(), before, "re-entering stamps nothing");
}

#[test]
fn an_unchanged_value_is_not_stamped_again() {
    let _c = capture_exclusive();
    let span = info_span!(parent: None, "stamp-same");
    span.in_scope(|| context::add("action_id", "A"));
    let before = stamp_count();
    span.in_scope(|| context::add("action_id", "A"));
    assert_eq!(stamp_count(), before);
}

#[test]
fn two_hundred_adds_export_the_last_value_and_a_later_record_survives() {
    let _c = capture_exclusive();
    let span = info_span!(parent: None, "stamp-many", status = tracing::field::Empty);
    span.in_scope(|| {
        for i in 0..200 {
            context::add("turn_id", format!("t{i}"));
        }
    });
    span.record("status", 404);
    drop(span);
    let exported = spans()
        .into_iter()
        .find(|s| s.name == "stamp-many")
        .expect("exported");
    assert_eq!(span_attr(&exported, "turn_id").as_deref(), Some("t199"));
    assert_eq!(span_attr(&exported, "status").as_deref(), Some("404"));
}
