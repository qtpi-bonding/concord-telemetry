//! Tracing-aware Tokio channels and task spawning.

use std::future::Future;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;
use tracing::Instrument;
use tracing::Span;

pub struct TracedSender<T> {
    inner: mpsc::Sender<(T, Span)>,
}

pub struct TracedReceiver<T> {
    inner: mpsc::Receiver<(T, Span)>,
}

/// Creates a bounded channel whose messages carry the sending span.
pub fn traced<T>(cap: usize) -> (TracedSender<T>, TracedReceiver<T>) {
    let (sender, receiver) = mpsc::channel(cap);
    (
        TracedSender { inner: sender },
        TracedReceiver { inner: receiver },
    )
}

impl<T> Clone for TracedSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> TracedSender<T> {
    /// Sends `v`, recording the span current at the call site.
    pub async fn send(&self, v: T) -> Result<(), SendError<T>> {
        let span = Span::current();
        self.inner
            .send((v, span))
            .await
            .map_err(|SendError((value, _))| SendError(value))
    }
}

impl<T> TracedReceiver<T> {
    /// Receives a value and the span in which its sender sent it.
    pub async fn recv(&mut self) -> Option<(T, Span)> {
        self.inner.recv().await
    }
}

/// Spawns a task in the currently entered span.
pub fn spawn_traced<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(f.in_current_span())
}
