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

    /// Attempts to send `v` without waiting, recording the span current at the call site.
    ///
    /// Returns `TrySendError::Full` if the channel is at capacity and
    /// `TrySendError::Closed` if the receiver has been dropped or closed.
    /// The returned error carries the original value, not the span.
    pub fn try_send(&self, v: T) -> Result<(), TrySendError<T>> {
        let span = Span::current();
        self.inner.try_send((v, span)).map_err(|err| match err {
            TrySendError::Full((value, _)) => TrySendError::Full(value),
            TrySendError::Closed((value, _)) => TrySendError::Closed(value),
        })
    }

    /// Waits for a slot in the channel and reserves it as a [`TracedPermit`].
    ///
    /// The span is recorded when the permit's `send` is called, not here.
    /// Dropping the permit without sending releases the reserved slot.
    /// This method is cancel-safe.
    pub async fn reserve(&self) -> Result<TracedPermit<'_, T>, SendError<()>> {
        self.inner
            .reserve()
            .await
            .map(|permit| TracedPermit { inner: permit })
    }

    /// Completes when the receiver has been dropped or closed.
    pub async fn closed(&self) {
        self.inner.closed().await
    }

    /// Returns `true` if the receiver has been dropped or closed.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

/// A reserved slot in a [`TracedSender`] channel.
///
/// Dropping the permit without calling [`TracedPermit::send`] frees the slot.
pub struct TracedPermit<'a, T> {
    inner: mpsc::Permit<'a, (T, Span)>,
}

impl<T> TracedPermit<'_, T> {
    /// Sends `v` through the reserved slot, recording the span current at the call to `send`.
    pub fn send(self, v: T) {
        let span = Span::current();
        self.inner.send((v, span));
    }
}

impl<T> TracedReceiver<T> {
    /// Receives a value and the span in which its sender sent it.
    pub async fn recv(&mut self) -> Option<(T, Span)> {
        self.inner.recv().await
    }

    /// Polls for the next value and its sending span, registering the task waker if none is ready.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<(T, Span)>> {
        self.inner.poll_recv(cx)
    }

    /// Closes the receiving half, preventing new messages from being sent while
    /// still allowing buffered messages to be received.
    pub fn close(&mut self) {
        self.inner.close()
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
