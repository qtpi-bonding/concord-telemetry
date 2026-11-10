use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Run a potentially blocking operation without making the caller wait beyond `d`.
pub(crate) fn run_with_deadline<F>(d: Duration, f: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = sender.send(());
    });
    receiver.recv_timeout(d).is_ok()
}

/// What `run_exclusive` did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Exclusive {
    /// It ran to completion within the deadline.
    Done,
    /// It was started and is still running past the deadline.
    TimedOut,
    /// Another run is in flight; this one did not start.
    InFlight,
}

/// `run_with_deadline`, with at most one run in flight on `in_flight`. The
/// flag clears when the run finishes, not when the deadline passes, so a
/// later call after a slow one still runs once it has completed.
pub(crate) fn run_exclusive<F>(in_flight: &'static AtomicBool, d: Duration, f: F) -> Exclusive
where
    F: FnOnce() + Send + 'static,
{
    if in_flight.swap(true, Ordering::AcqRel) {
        return Exclusive::InFlight;
    }
    struct Clear(&'static AtomicBool);
    impl Drop for Clear {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let clear = Clear(in_flight);
    if run_with_deadline(d, move || {
        let _clear = clear;
        f();
    }) {
        Exclusive::Done
    } else {
        Exclusive::TimedOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn at_most_one_run_is_in_flight_and_a_later_one_runs_after_it() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let (release, held) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        let first = std::thread::spawn(move || {
            run_exclusive(&FLAG, Duration::from_millis(50), move || {
                started.send(()).expect("started");
                let _ = held.recv();
            })
        });
        running.recv().expect("the first run started");
        // Still in flight: a second panic's flush does not start another.
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        assert_eq!(
            run_exclusive(&FLAG, Duration::from_millis(50), move || {
                flag.store(true, Ordering::SeqCst)
            }),
            Exclusive::InFlight
        );
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(first.join().expect("first"), Exclusive::TimedOut);
        release.send(()).expect("release");
        // Once it finishes, a later call (a fatal panic after a caught one) runs.
        let mut outcome = Exclusive::InFlight;
        for _ in 0..200 {
            let flag = Arc::clone(&ran);
            outcome = run_exclusive(&FLAG, Duration::from_secs(5), move || {
                flag.store(true, Ordering::SeqCst)
            });
            if outcome != Exclusive::InFlight {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(outcome, Exclusive::Done);
        assert!(ran.load(Ordering::SeqCst));
    }
}
