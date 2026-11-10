use std::sync::mpsc;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
    /// Another run stayed in flight for the whole deadline; this one did not start.
    InFlight,
}

/// At most one run in flight.
pub(crate) struct Gate {
    busy: Mutex<bool>,
    cleared: Condvar,
}

impl Gate {
    pub(crate) const fn new() -> Self {
        Self {
            busy: Mutex::new(false),
            cleared: Condvar::new(),
        }
    }

    fn set_busy(&self, busy: bool) {
        *self.busy.lock().unwrap_or_else(PoisonError::into_inner) = busy;
        self.cleared.notify_all();
    }
}

/// `run_with_deadline`, with at most one run in flight on `gate`. A caller that
/// finds one in flight waits for it to finish within `d` (a fatal panic after
/// a caught one must still flush its own record), then runs; if it is still in
/// flight at the deadline, nothing is started. The gate clears when the run
/// finishes, not when the deadline passes.
pub(crate) fn run_exclusive<F>(gate: &'static Gate, d: Duration, f: F) -> Exclusive
where
    F: FnOnce() + Send + 'static,
{
    let started = Instant::now();
    {
        let mut busy = gate.busy.lock().unwrap_or_else(PoisonError::into_inner);
        while *busy {
            let Some(left) = d.checked_sub(started.elapsed()).filter(|l| !l.is_zero()) else {
                return Exclusive::InFlight;
            };
            busy = gate
                .cleared
                .wait_timeout(busy, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        *busy = true;
    }
    struct Clear(&'static Gate);
    impl Drop for Clear {
        fn drop(&mut self) {
            self.0.set_busy(false);
        }
    }
    let clear = Clear(gate);
    let left = d.saturating_sub(started.elapsed());
    if run_with_deadline(left, move || {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};

    #[test]
    fn a_second_run_waits_for_the_first_within_its_deadline_then_runs() {
        static FLAG: Gate = Gate::new();
        let (release, held) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        let first = std::thread::spawn(move || {
            run_exclusive(&FLAG, Duration::from_millis(50), move || {
                started.send(()).expect("started");
                let _ = held.recv();
            })
        });
        running.recv().expect("the first run started");
        assert_eq!(first.join().expect("first"), Exclusive::TimedOut);

        // A second panic while the first flush is in flight, and that flush
        // finishing within the second's deadline: the second still flushes.
        let ran = Arc::new(AtomicBool::new(false));
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            release.send(()).expect("release");
        });
        let flag = Arc::clone(&ran);
        let outcome = run_exclusive(&FLAG, Duration::from_secs(10), move || {
            flag.store(true, Ordering::SeqCst)
        });
        releaser.join().expect("releaser");
        assert_eq!(outcome, Exclusive::Done);
        assert!(ran.load(Ordering::SeqCst));

        // A flush that never finishes is not waited for beyond the deadline.
        let (_keep, held) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            run_exclusive(&FLAG, Duration::from_millis(10), move || {
                started.send(()).expect("started");
                let _ = held.recv();
            })
        });
        running.recv().expect("stuck run started");
        let began = std::time::Instant::now();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        let outcome = run_exclusive(&FLAG, Duration::from_millis(100), move || {
            flag.store(true, Ordering::SeqCst)
        });
        assert_eq!(outcome, Exclusive::InFlight);
        assert!(!ran.load(Ordering::SeqCst));
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}
