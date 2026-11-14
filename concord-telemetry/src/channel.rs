//! Tracing-aware Tokio channels and task spawning.

use std::future::Future;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::{SendError, TrySendError};
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

    /// Not implemented yet (red-test stub).
    pub fn try_send(&self, _v: T) -> Result<(), TrySendError<T>> {
        unimplemented!("try_send")
    }

    /// Not implemented yet (red-test stub).
    pub async fn reserve(&self) -> Result<TracedPermit<'_, T>, SendError<()>> {
        unimplemented!("reserve")
    }

    /// Not implemented yet (red-test stub).
    pub async fn closed(&self) {
        unimplemented!("closed")
    }

    /// Not implemented yet (red-test stub).
    pub fn is_closed(&self) -> bool {
        unimplemented!("is_closed")
    }
}

/// Not implemented yet (red-test stub).
pub struct TracedPermit<'a, T> {
    _inner: std::marker::PhantomData<&'a T>,
}

impl<T> TracedPermit<'_, T> {
    /// Not implemented yet (red-test stub).
    pub fn send(self, _v: T) {
        unimplemented!("permit send")
    }
}

impl<T> TracedReceiver<T> {
    /// Receives a value and the span in which its sender sent it.
    pub async fn recv(&mut self) -> Option<(T, Span)> {
        self.inner.recv().await
    }

    /// Not implemented yet (red-test stub).
    pub fn poll_recv(&mut self, _cx: &mut Context<'_>) -> Poll<Option<(T, Span)>> {
        unimplemented!("poll_recv")
    }

    /// Not implemented yet (red-test stub).
    pub fn close(&mut self) {
        unimplemented!("close")
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
