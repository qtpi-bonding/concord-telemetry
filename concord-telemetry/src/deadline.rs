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
