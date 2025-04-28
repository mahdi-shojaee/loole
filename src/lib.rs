//! An async/sync multi-producer multi-consumer channel. Multiple threads can send and receive
//! messages on the channel at the same time and each message will be received by only one thread.
//!
//! Producers can send and consumers can receive messages asynchronously or synchronously:
//!
//! There are two types of channels: bounded and unbounded.
//!
//! 1. [Bounded][`bounded()`] channel with limited capacity.
//! 2. [Unbounded][`unbounded()`] channel with unlimited capacity.
//!
//! A channel has two sides: the [`Sender`] side and the [`Receiver`] side. Both sides are cloneable, meaning
//! that they can be copied and shared among multiple threads. This allows you to have multiple
//! threads sending and receiving messages on the same channel.
//!
//! When all [`Sender`]s or all [`Receiver`]s are dropped, the channel becomes closed. This means that no
//! more messages can be sent, but remaining messages can still be received.
//!
//! The channel can also be closed manually by calling Sender::close() or Receiver::close().
//!
//! # Examples
//!
//! ```
//! let (tx, rx) = loole::unbounded();
//!
//! std::thread::spawn(move || {
//!     for i in 0..10 {
//!         tx.send(i).unwrap();
//!     }
//! });
//!
//! let mut sum = 0;
//! while let Ok(i) = rx.recv() {
//!     sum += i;
//! }
//!
//! assert_eq!(sum, (0..10).sum());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod mutex;
mod queue;
mod signal;

use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::ready;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures_core::Stream;
use futures_sink::Sink;

use mutex::MutexGuard;
use queue::Queue;

use crate::mutex::Mutex;
use crate::signal::{Signal, SyncSignal};

/// An error that occurs when trying to receive a value from a channel after all senders have been
/// dropped and there are no more messages in the channel.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RecvError {
    /// No further messages can be received because all senders have been dropped and there are no messages
    /// waiting in the channel.
    Disconnected,
}

impl std::error::Error for RecvError {}

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecvError::Disconnected => f.write_str("receiving on a closed channel"),
        }
    }
}

/// An error that occurs when trying to send a value on a channel after all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

impl<T> std::error::Error for SendError<T> {}

impl<T> SendError<T> {
    /// Consumes the error, returning the message that failed to send.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("sending on a closed channel")
    }
}

impl<T> std::fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendError(..)")
    }
}

/// An error that occurs when trying to send a value to a channel:
///
/// * When the channel is full.
/// * When all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TrySendError<T> {
    /// The message cannot be sent because the channel is full.
    Full(T),
    /// All receivers have been dropped, so the message cannot be received.
    Disconnected(T),
}

impl<T> TrySendError<T> {
    /// Consume the error and return the message that failed to send.
    pub fn into_inner(self) -> T {
        match self {
            Self::Full(msg) | Self::Disconnected(msg) => msg,
        }
    }
}

impl<T> std::fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            TrySendError::Full(..) => f.write_str("Full(..)"),
            TrySendError::Disconnected(..) => f.write_str("Disconnected(..)"),
        }
    }
}

impl<T> std::fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrySendError::Full(..) => f.write_str("sending on a full channel"),
            TrySendError::Disconnected(..) => f.write_str("sending on a closed channel"),
        }
    }
}

impl<T> std::error::Error for TrySendError<T> {}

impl<T> From<SendError<T>> for TrySendError<T> {
    fn from(err: SendError<T>) -> Self {
        match err {
            SendError(item) => Self::Disconnected(item),
        }
    }
}

/// An error that occurs when trying to receive a value from a channel when there are no messages in the channel.
/// If there are no messages in the channel and all senders are dropped, then the `Disconnected` error will be
/// returned.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryRecvError {
    /// An error that occurs when trying to receive a value from an empty channel.
    Empty,
    /// The channel has been closed because all senders have been dropped and there are no more messages waiting
    /// in the channel.
    Disconnected,
}

impl std::fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TryRecvError::Empty => f.write_str("receiving on an empty channel"),
            TryRecvError::Disconnected => f.write_str("channel is empty and closed"),
        }
    }
}

impl std::error::Error for TryRecvError {}

impl From<RecvError> for TryRecvError {
    fn from(err: RecvError) -> Self {
        match err {
            RecvError::Disconnected => Self::Disconnected,
        }
    }
}

/// An error that may be returned when attempting to receive a value on a channel with a timeout
/// and no value is received within the timeout period, or when all senders have been dropped and
/// there are no more values left in the channel.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RecvTimeoutError {
    /// The operation timed out while waiting for a message to be received.
    Timeout,
    /// The channel is empty and all senders have been dropped, so no further messages can be received.
    Disconnected,
}

impl std::error::Error for RecvTimeoutError {}

impl std::fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecvTimeoutError::Timeout => f.write_str("timed out waiting on a channel"),
            RecvTimeoutError::Disconnected => f.write_str("channel is empty and closed"),
        }
    }
}

/// An error that may be emitted when sending a value into a channel on a sender with a timeout.
///
/// This error can occur when either:
///
/// * The send operation times out before the value is successfully sent.
/// * All receivers of the channel are dropped before the value is successfully sent.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SendTimeoutError<T> {
    /// A timeout occurred when attempting to send the message.
    Timeout(T),
    /// The message cannot be sent because all channel receivers were dropped.
    Disconnected(T),
}

impl<T> std::error::Error for SendTimeoutError<T> {}

impl<T> SendTimeoutError<T> {
    /// Consumes the error, returning the message that failed to send.
    pub fn into_inner(self) -> T {
        match self {
            Self::Timeout(msg) | Self::Disconnected(msg) => msg,
        }
    }
}

impl<T> std::fmt::Debug for SendTimeoutError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendTimeoutError(..)")
    }
}

impl<T> std::fmt::Display for SendTimeoutError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendTimeoutError::Timeout(..) => f.write_str("timed out sending on a full channel"),
            SendTimeoutError::Disconnected(..) => f.write_str("sending on a closed channel"),
        }
    }
}

impl<T> From<SendError<T>> for SendTimeoutError<T> {
    fn from(value: SendError<T>) -> Self {
        SendTimeoutError::Disconnected(value.0)
    }
}

/// An iterator over the msgs received synchronously from a channel.
pub struct Iter<'a, T> {
    receiver: &'a Receiver<T>,
}

/// An non-blocking iterator over the msgs received synchronously from a channel.
pub struct TryIter<'a, T> {
    receiver: &'a Receiver<T>,
}

/// An owned iterator over the msgs received synchronously from a channel.
pub struct IntoIter<T> {
    receiver: Receiver<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

#[derive(Debug)]
struct SharedState<T> {
    pending_recvs: Queue<Signal>,
    pending_sends: Queue<(T, Option<Signal>)>,
    queue: Queue<T>,
    closed: bool,
    cap: Option<usize>,
    next_id: usize,
}

impl<T> SharedState<T> {
    fn new(cap: Option<usize>) -> Self {
        let pending_sends = cap.map_or_else(Queue::new, Queue::with_capacity);
        Self {
            pending_recvs: Queue::new(),
            pending_sends,
            queue: Queue::new(),
            closed: false,
            cap,
            next_id: 1,
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    fn is_full(&self) -> bool {
        Some(self.len()) == self.cap
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get_next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn close(&mut self) -> bool {
        let was_closed = self.closed;
        self.closed = true;
        for (_, s) in self.pending_recvs.iter() {
            s.wake_by_ref();
        }
        for (_, (_, s)) in self.pending_sends.iter() {
            if let Some(s) = s {
                s.wake_by_ref();
            }
        }
        !was_closed
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

enum TrySendResult<'a, T> {
    Ok,
    Disconnected(T),
    Full(T, MutexGuard<'a, SharedState<T>>),
}

#[inline(always)]
fn try_send<T>(m: T, id: usize, mut guard: MutexGuard<'_, SharedState<T>>) -> TrySendResult<T> {
    if guard.closed {
        return TrySendResult::Disconnected(m);
    }
    if !guard.is_full() {
        guard.queue.enqueue(id, m);

        let pending_recvs = std::mem::take(&mut guard.pending_recvs);
        drop(guard);
        for (_, s) in pending_recvs.iter() {
            s.wake_by_ref();
        }
        
        return TrySendResult::Ok;
    } else if guard.cap == Some(0) {
        if let Some((_, s)) = guard.pending_recvs.dequeue() {
            guard.pending_sends.enqueue(id, (m, None));
            drop(guard);
            s.wake();
            return TrySendResult::Ok;
        }
    }
    TrySendResult::Full(m, guard)
}

enum TryRecvResult<'a, T> {
    Ok(T),
    Disconnected,
    Empty(MutexGuard<'a, SharedState<T>>),
}

#[inline(always)]
fn try_recv<T>(mut guard: MutexGuard<'_, SharedState<T>>) -> TryRecvResult<T> {
    if let Some((_, m)) = guard.queue.dequeue() {
        if let Some((id, (m, s))) = guard.pending_sends.dequeue() {
            guard.queue.enqueue(id, m);
            if let Some(s) = s {
                drop(guard);
                s.wake();
            }
        }
        return TryRecvResult::Ok(m);
    } else if guard.cap == Some(0) {
        if let Some((_, (m, s))) = guard.pending_sends.dequeue() {
            if let Some(s) = s {
                drop(guard);
                s.wake();
            }
            return TryRecvResult::Ok(m);
        }
    }
    if guard.closed {
        return TryRecvResult::Disconnected;
    }
    TryRecvResult::Empty(guard)
}

/// A future that sends a value into a channel.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[derive(Debug)]
pub struct SendFuture<T> {
    sender: Sender<T>,
    msg: MessageOrId<T>,
}

impl<T> SendFuture<T> {
    /// See [`Sender::is_closed`].
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// See [`Sender::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.sender.is_empty()
    }

    /// See [`Sender::is_full`].
    pub fn is_full(&self) -> bool {
        self.sender.is_full()
    }

    /// See [`Sender::len`].
    pub fn len(&self) -> usize {
        self.sender.len()
    }

    /// See [`Sender::capacity`].
    pub fn capacity(&self) -> Option<usize> {
        self.sender.capacity()
    }
}

/// A sink that allows sending values into a channel.
///
/// Can be created via [`Sender::sink`] or [`Sender::into_sink`].
pub struct SendSink<T>(SendFuture<T>);

impl<T> SendSink<T> {
    /// Returns a clone of a sending half of the channel of this sink.
    pub fn sender(&self) -> &Sender<T> {
        &self.0.sender
    }

    /// See [`Sender::is_closed`].
    pub fn is_closed(&self) -> bool {
        self.0.sender.is_closed()
    }

    /// See [`Sender::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.0.sender.is_empty()
    }

    /// See [`Sender::is_full`].
    pub fn is_full(&self) -> bool {
        self.0.sender.is_full()
    }

    /// See [`Sender::len`].
    pub fn len(&self) -> usize {
        self.0.sender.len()
    }

    /// See [`Sender::capacity`].
    pub fn capacity(&self) -> Option<usize> {
        self.0.sender.capacity()
    }

    /// Returns whether the SendSinks are belong to the same channel.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.0.sender.same_channel(&other.0.sender)
    }
}

impl<T> Debug for SendSink<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendSink").finish()
    }
}

impl<T> Sink<T> for SendSink<T> {
    type Error = SendError<T>;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.0.msg = MessageOrId::Message(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let MessageOrId::Message(_) = self.0.msg {
            ready!(Pin::new(&mut self.0).poll(cx))?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let MessageOrId::Message(_) = self.0.msg {
            ready!(Pin::new(&mut self.0).poll(cx))?;
        }
        self.0.sender.close();
        Poll::Ready(Ok(()))
    }
}

impl<T> Clone for SendSink<T> {
    fn clone(&self) -> SendSink<T> {
        SendSink(SendFuture {
            sender: self.0.sender.clone(),
            msg: MessageOrId::Invalid,
        })
    }
}

#[derive(Debug)]
enum MessageOrId<T> {
    Message(T),
    Id(usize),
    Invalid,
}

impl<T> MessageOrId<T> {
    fn take(&mut self) -> Self {
        std::mem::replace(self, Self::Invalid)
    }
}

impl<T> std::marker::Unpin for SendFuture<T> {}

impl<T> Future for SendFuture<T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let m = match self.msg.take() {
            MessageOrId::Message(m) => m,
            MessageOrId::Id(id) => {
                let mut guard = self.sender.inner.shared_state.lock();
                if guard.closed {
                    if let Some((_, (m, Some(_)))) = guard.pending_sends.remove(id) {
                        return Poll::Ready(Err(SendError(m)));
                    }
                }
                // Check if the message has been sent
                if !guard.pending_sends.contains(id) {
                    return Poll::Ready(Ok(()));
                }
                // Message is still pending, update the waker and return Pending
                let s = if let Some((_, Some(s))) = guard.pending_sends.get(id) {
                    Some(s.clone())
                } else {
                    None
                };
                drop(guard);
                if let Some(s) = s {
                    s.wake();
                }
                self.msg = MessageOrId::Id(id);
                return Poll::Pending;
            }
            MessageOrId::Invalid => panic!("Future polled after completion"),
        };
        let mut guard = self.sender.inner.shared_state.lock();
        let id = guard.get_next_id();
        let (m, mut guard) = match try_send(m, id, guard) {
            TrySendResult::Ok => return Poll::Ready(Ok(())),
            TrySendResult::Disconnected(m) => return Poll::Ready(Err(SendError(m))),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        guard
            .pending_sends
            .enqueue(id, (m, Some(cx.waker().clone().into())));
        let opt = guard.pending_recvs.dequeue();
        drop(guard);
        if let Some((_, s)) = opt {
            s.wake();
        }
        self.msg = MessageOrId::Id(id);
        Poll::Pending
    }
}

/// A future that allows asynchronously receiving a message.
/// This future will resolve to a message when a message is available on the channel,
/// or to an error if the channel is closed.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[derive(Debug)]
pub struct RecvFuture<T> {
    id: usize,
    receiver: Receiver<T>,
}

impl<T> RecvFuture<T> {
    /// See [`Receiver::is_closed`].
    pub fn is_closed(&self) -> bool {
        self.receiver.is_closed()
    }

    /// See [`Receiver::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }

    /// See [`Receiver::is_full`].
    pub fn is_full(&self) -> bool {
        self.receiver.is_full()
    }

    /// See [`Receiver::len`].
    pub fn len(&self) -> usize {
        self.receiver.len()
    }

    /// See [`Receiver::capacity`].
    pub fn capacity(&self) -> Option<usize> {
        self.receiver.capacity()
    }
}

impl<T> Future for RecvFuture<T> {
    type Output = Result<T, RecvError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut guard = match try_recv(self.receiver.inner.shared_state.lock()) {
            TryRecvResult::Ok(r) => return Poll::Ready(Ok(r)),
            TryRecvResult::Disconnected => return Poll::Ready(Err(RecvError::Disconnected)),
            TryRecvResult::Empty(guard) => guard,
        };
        if guard.closed {
            return Poll::Ready(Err(RecvError::Disconnected));
        }
        if !guard.pending_recvs.contains(self.id) {
            guard
                .pending_recvs
                .enqueue(self.id, cx.waker().clone().into());
        }
        Poll::Pending
    }
}

impl<T> Drop for RecvFuture<T> {
    fn drop(&mut self) {
        let mut guard = self.receiver.inner.shared_state.lock();
        guard.pending_recvs.remove(self.id);
    }
}

struct SenderInner<T> {
    shared_state: Arc<Mutex<SharedState<T>>>,
    send_count: AtomicUsize,
    next_id: AtomicUsize,
}

/// The sending half of a channel.
pub struct Sender<T> {
    inner: Arc<SenderInner<T>>,
}

impl<T> std::fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender").finish()
    }
}

impl<T> Clone for Sender<T> {
    /// Clones this sender. A [`Sender`] acts as a handle to the sending end of a channel. The remaining
    /// contents of the channel will only be cleaned up when all senders and the receiver have been dropped.
    fn clone(&self) -> Self {
        self.inner.send_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Sender<T> {
    fn new(shared_state: Arc<Mutex<SharedState<T>>>) -> Self {
        Self {
            inner: Arc::new(SenderInner {
                shared_state,
                send_count: AtomicUsize::new(1),
                next_id: AtomicUsize::new(1),
            })
        }
    }

    fn get_next_id(&self) -> usize {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// It returns an error if the channel is bounded and full, or if all receivers have been dropped.
    /// If the channel is unbounded, the method behaves the same as the [`Sender::send`] method.
    ///
    /// This method is useful for avoiding deadlocks. If you are sending a value into a channel and
    /// you are not sure if the channel is full or if all receivers have been dropped, you can use
    /// this method instead of the [`Sender::send`] method. If this method returns an error, you can
    /// take appropriate action, such as retrying the send operation later or buffering the value
    /// until you can send it successfully.
    pub fn try_send(&self, m: T) -> Result<(), TrySendError<T>> {
        match try_send(m, self.get_next_id(), self.inner.shared_state.lock()) {
            TrySendResult::Ok => Ok(()),
            TrySendResult::Disconnected(m) => Err(TrySendError::Disconnected(m)),
            TrySendResult::Full(m, _) => Err(TrySendError::Full(m)),
        }
    }

    /// Asynchronously send a value into the channel, it will return a future that completes when the
    /// value has been successfully sent, or when an error has occurred.
    ///
    /// The method returns an error if all receivers on the channel have been dropped.
    /// If the channel is bounded and is full, the returned future will yield to the async runtime.
    pub fn send_async(&self, m: T) -> SendFuture<T> {
        SendFuture {
            sender: self.clone(),
            msg: MessageOrId::Message(m),
        }
    }

    /// Sends a value into the channel. Returns an error if all receivers have been dropped, or if
    /// the channel is bounded and is full and no receivers are available.
    pub fn send(&self, m: T) -> Result<(), SendError<T>> {
        let id = self.get_next_id();
        let (m, mut guard) = match try_send(m, id, self.inner.shared_state.lock()) {
            TrySendResult::Ok => return Ok(()),
            TrySendResult::Disconnected(m) => return Err(SendError(m)),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        let sync_signal = SyncSignal::new();

        guard
            .pending_sends
            .enqueue(id, (m, Some(sync_signal.clone().into())));
        drop(guard);
        loop {
            sync_signal.wait();
            let mut guard = self.inner.shared_state.lock();
            if guard.closed {
                if let Some((_, (m, Some(_)))) = guard.pending_sends.remove(id) {
                    return Err(SendError(m));
                }
            }
            if !guard.pending_sends.contains(id) {
                break;
            }
        }
        Ok(())
    }

    /// Attempts to send a value into the channel.
    ///
    /// If all receivers have been dropped or the timeout has expired, this method will return
    /// an error. If the channel is bounded and is full, this method will block until space is
    /// available, the timeout has expired, or all receivers have been dropped.
    pub fn send_timeout(&self, m: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        let id = self.get_next_id();
        let (m, mut guard) = match try_send(m, id, self.inner.shared_state.lock()) {
            TrySendResult::Ok => return Ok(()),
            TrySendResult::Disconnected(m) => return Err(SendTimeoutError::Disconnected(m)),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        let sync_signal = SyncSignal::new();
        guard
            .pending_sends
            .enqueue(id, (m, Some(sync_signal.clone().into())));
        drop(guard);
        loop {
            let _ = sync_signal.wait_timeout(timeout);
            let mut guard = self.inner.shared_state.lock();
            if let Some((_, (m, Some(_)))) = guard.pending_sends.remove(id) {
                if guard.closed {
                    return Err(SendTimeoutError::Disconnected(m));
                }
                return Err(SendTimeoutError::Timeout(m));
            }
            if !guard.pending_sends.contains(id) {
                break;
            }
        }
        Ok(())
    }

    /// Sends a value into the channel, returning an error if the channel is full and the
    /// deadline has passed, or if all receivers have been dropped.
    pub fn send_deadline(&self, m: T, deadline: Instant) -> Result<(), SendTimeoutError<T>> {
        self.send_timeout(m, deadline.checked_duration_since(Instant::now()).unwrap())
    }

    /// Returns `true` if the two senders belong to the same channel, and `false` otherwise.
    pub fn same_channel(&self, other: &Sender<T>) -> bool {
        Arc::ptr_eq(&self.inner.shared_state, &other.inner.shared_state)
    }

    /// Returns the number of messages currently in the channel.
    ///
    /// This function is useful for determining how many messages are waiting to be processed
    /// by consumers, or for implementing backpressure mechanisms.
    pub fn len(&self) -> usize {
        self.inner.shared_state.lock().len()
    }

    /// Returns the capacity of the channel, if it is bounded. Otherwise, returns `None`.
    pub fn capacity(&self) -> Option<usize> {
        self.inner.shared_state.lock().cap
    }

    /// Returns true if the channel is empty.
    ///
    /// Note: Zero-capacity channels are always empty.
    pub fn is_empty(&self) -> bool {
        self.inner.shared_state.lock().is_empty()
    }

    /// Returns true if the channel is full.
    ///
    /// Note: Zero-capacity channels are always full.
    pub fn is_full(&self) -> bool {
        self.inner.shared_state.lock().is_full()
    }

    /// Closes the channel.
    ///
    /// Returns true only if this call actively closed the channel, which was previously open.
    ///
    /// The remaining messages can still be received.
    pub fn close(&self) -> bool {
        self.inner.shared_state.lock().close()
    }

    /// Returns true if the channel is closed.
    pub fn is_closed(&self) -> bool {
        self.inner.shared_state.lock().is_closed()
    }

    /// Returns a sink that can be used to send values into the channel.
    pub fn sink(&self) -> SendSink<T> {
        SendSink(SendFuture {
            sender: self.clone(),
            msg: MessageOrId::Invalid,
        })
    }

    /// Converts this sender into a sink that can be used to send values into the channel.
    pub fn into_sink(self) -> SendSink<T> {
        SendSink(SendFuture {
            sender: self,
            msg: MessageOrId::Invalid,
        })
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let _ = self
            .inner
            .send_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                let mut count = c;
                if count > 0 {
                    count -= 1;
                    if count == 0 {
                        self.inner.shared_state.lock().close();
                    }
                }
                Some(count)
            });
    }
}

struct ReceiverInner<T> {
    shared_state: Arc<Mutex<SharedState<T>>>,
    recv_count: AtomicUsize,
    next_id: AtomicUsize,
}

/// The receiving end of a channel.
///
/// Note: Cloning the receiver *does not* turn this channel into a broadcast channel.
/// Each message will only be received by a single receiver. This is useful for
/// implementing work stealing for concurrent programs.
pub struct Receiver<T> {
    inner: Arc<ReceiverInner<T>>,
}

impl<T> std::fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

impl<T> Clone for Receiver<T> {
    /// Clone this receiver. [`Receiver`] acts as a handle to the ending a channel. Remaining
    /// channel contents will only be cleaned up when all senders and the receiver have been
    /// dropped.
    ///
    /// Note: Cloning the receiver *does not* turn this channel into a broadcast channel.
    /// Each message will only be received by a single receiver. This is useful for
    /// implementing work stealing for concurrent programs.
    fn clone(&self) -> Self {
        self.inner.recv_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Receiver<T> {
    fn new(shared_state: Arc<Mutex<SharedState<T>>>) -> Self {
        Self {
            inner: Arc::new(ReceiverInner {
                shared_state,
                recv_count: AtomicUsize::new(1),
                next_id: AtomicUsize::new(1),
            }),
        }
    }

    fn get_next_id(&self) -> usize {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Attempts to receive a value from the channel associated with this receiver, returning an error if
    /// the channel is empty or if all senders have been dropped.
    ///
    /// This method will block until a value is available on the channel, or until the channel is empty
    /// or all senders have been dropped.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match try_recv(self.inner.shared_state.lock()) {
            TryRecvResult::Ok(m) => Ok(m),
            TryRecvResult::Disconnected => Err(TryRecvError::Disconnected),
            TryRecvResult::Empty(_) => Err(TryRecvError::Empty),
        }
    }

    /// Asynchronously receive a value from the channel, returning an error if all senders have been dropped.
    /// If the channel is empty, the returned future will yield to the async runtime.
    ///
    /// This method returns a future that will be resolved with the value received from the channel,
    /// or with an error if the channel is closed.
    pub fn recv_async(&self) -> RecvFuture<T> {
        RecvFuture {
            id: self.get_next_id(),
            receiver: self.clone(),
        }
    }

    /// Wait for an incoming value from the channel associated with this receiver. If all senders have been
    /// dropped and there are no more messages in the channel, this method will return an error.
    pub fn recv(&self) -> Result<T, RecvError> {
        loop {
            let mut guard = match try_recv(self.inner.shared_state.lock()) {
                TryRecvResult::Ok(r) => return Ok(r),
                TryRecvResult::Disconnected => return Err(RecvError::Disconnected),
                TryRecvResult::Empty(guard) => guard,
            };
            let id = self.get_next_id();
            let sync_signal = SyncSignal::new();
            guard.pending_recvs.enqueue(id, sync_signal.clone().into());
            drop(guard);
            sync_signal.wait();
        }
    }

    /// Receives a value from the channel associated with this receiver, blocking the current thread
    /// until a value is available or the timeout expires.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let start_time = Instant::now();
        let mut timeout_remaining = timeout;
        loop {
            let mut guard = match try_recv(self.inner.shared_state.lock()) {
                TryRecvResult::Ok(r) => return Ok(r),
                TryRecvResult::Disconnected => return Err(RecvTimeoutError::Disconnected),
                TryRecvResult::Empty(guard) => guard,
            };
            if guard.closed {
                return Err(RecvTimeoutError::Disconnected);
            }
            let id = self.get_next_id();
            let sync_signal = SyncSignal::new();
            guard.pending_recvs.enqueue(id, sync_signal.clone().into());
            drop(guard);
            let _ = sync_signal.wait_timeout(timeout_remaining);
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                let mut guard = self.inner.shared_state.lock();
                guard.pending_recvs.remove(id);
                drop(guard);
                return Err(RecvTimeoutError::Timeout);
            }
            timeout_remaining = timeout - elapsed;
        }
    }

    /// Receives a value from the channel associated with this receiver, blocking the current thread
    /// until a value is available or the deadline has passed.
    pub fn recv_deadline(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        self.recv_timeout(deadline.checked_duration_since(Instant::now()).unwrap())
    }

    /// Take all msgs currently sitting in the channel and produce an iterator over them. Unlike
    /// `try_iter`, the iterator will not attempt to fetch any more values from the channel once
    /// the function has been called.
    pub fn drain(&self) -> Drain<T> {
        let mut guard = self.inner.shared_state.lock();
        let queue = std::mem::take(&mut guard.queue);
        let n = guard
            .cap
            .map_or(0, |cap| cap.min(guard.pending_sends.len()));
        for _ in 0..n {
            if let Some((id, (m, mut s))) = guard.pending_sends.dequeue() {
                guard.queue.enqueue(id, m);
                if let Some(s) = s.take() {
                    s.wake();
                }
            }
        }
        Drain {
            queue,
            _phantom: PhantomData,
        }
    }

    /// Returns a blocking iterator over the values received on the channel. The iterator will finish
    /// iteration when all senders have been dropped.
    pub fn iter(&self) -> Iter<T> {
        Iter { receiver: self }
    }

    /// An iterator over the values received on the channel that finishes iteration when all senders
    /// have been dropped or the channel is empty.
    ///
    /// This iterator is non-blocking, meaning that it will not wait for the next value to be available
    /// if there is not one already. If there is no value available, the iterator will return `None`.
    pub fn try_iter(&self) -> TryIter<T> {
        TryIter { receiver: self }
    }

    /// Returns `true` if the two receivers belong to the same channel, and `false` otherwise.
    pub fn same_channel(&self, other: &Receiver<T>) -> bool {
        Arc::ptr_eq(&self.inner.shared_state, &other.inner.shared_state)
    }

    /// Returns the number of messages currently in the channel.
    ///
    /// This function is useful for determining how many messages are waiting to be processed
    /// by consumers, or for implementing backpressure mechanisms.
    pub fn len(&self) -> usize {
        self.inner.shared_state.lock().len()
    }

    /// Returns the capacity of the channel, if it is bounded. Otherwise, returns `None`.
    pub fn capacity(&self) -> Option<usize> {
        self.inner.shared_state.lock().cap
    }

    /// Returns true if the channel is empty.
    ///
    /// Note: Zero-capacity channels are always empty.
    pub fn is_empty(&self) -> bool {
        self.inner.shared_state.lock().is_empty()
    }

    /// Returns true if the channel is full.
    ///
    /// Note: Zero-capacity channels are always full.
    pub fn is_full(&self) -> bool {
        self.inner.shared_state.lock().is_full()
    }

    /// Closes the channel.
    ///
    /// Returns true only if this call actively closed the channel, which was previously open.
    ///
    /// The remaining messages can still be received.
    pub fn close(&self) -> bool {
        self.inner.shared_state.lock().close()
    }

    /// Returns true if the channel is closed.
    pub fn is_closed(&self) -> bool {
        self.inner.shared_state.lock().is_closed()
    }

    /// Returns a stream of messages from the channel.
    pub fn stream(&self) -> RecvStream<T> {
        RecvStream(RecvFuture {
            id: self.get_next_id(),
            receiver: self.clone(),
        })
    }

    /// Convert this receiver into a stream that allows asynchronously receiving messages from the channel.
    pub fn into_stream(self) -> RecvStream<T> {
        RecvStream(RecvFuture {
            id: self.get_next_id(),
            receiver: self,
        })
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let _ = self
            .inner
            .recv_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                let mut count = c;
                if count > 0 {
                    count -= 1;
                    if count == 0 {
                        self.inner.shared_state.lock().close();
                    }
                }
                Some(count)
            });
    }
}

/// An fixed-sized iterator over the msgs drained from a channel.
#[derive(Debug)]
pub struct Drain<'a, T> {
    queue: Queue<T>,
    /// A phantom field used to constrain the lifetime of this iterator. We do this because the
    /// implementation may change and we don't want to unintentionally constrain it. Removing this
    /// lifetime later is a possibility.
    _phantom: PhantomData<&'a ()>,
}

impl<'a, T> Iterator for Drain<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.queue.dequeue().map(|(_, x)| x)
    }
}

impl<'a, T> ExactSizeIterator for Drain<'a, T> {
    fn len(&self) -> usize {
        self.queue.len()
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

impl<'a, T> Iterator for TryIter<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.try_recv().ok()
    }
}

impl<'a, T> IntoIterator for &'a Receiver<T> {
    type Item = T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Iter { receiver: self }
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter { receiver: self }
    }
}

/// A stream which allows asynchronously receiving messages.
///
/// Can be created via [`Receiver::stream`] or [`Receiver::into_stream`].
pub struct RecvStream<T>(RecvFuture<T>);

impl<T> RecvStream<T> {
    /// See [`Receiver::is_closed`].
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// See [`Receiver::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// See [`Receiver::is_full`].
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// See [`Receiver::len`].
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// See [`Receiver::capacity`].
    pub fn capacity(&self) -> Option<usize> {
        self.0.capacity()
    }

    /// Returns whether the SendSinks are belong to the same channel.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.0.receiver.same_channel(&other.0.receiver)
    }
}

impl<T> std::fmt::Debug for RecvStream<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecvStream").finish()
    }
}

impl<T> Clone for RecvStream<T> {
    fn clone(&self) -> RecvStream<T> {
        RecvStream(RecvFuture {
            id: self.0.receiver.get_next_id(),
            receiver: self.0.receiver.clone(),
        })
    }
}

impl<T> Stream for RecvStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(item) => Poll::Ready(item.ok()),
        }
    }
}

fn channel<T>(cap: Option<usize>) -> (Sender<T>, Receiver<T>) {
    let shared_state = Arc::new(Mutex::new(SharedState::new(cap)));
    let sender = Sender::new(Arc::clone(&shared_state));
    let receiver = Receiver::new(shared_state);
    (sender, receiver)
}

/// Create a bounded channel with a limited maximum capacity.
///
/// Returns a tuple of a [`Sender`] and a [`Receiver`], which can be used to send and receive messages, respectively.
/// The channel has a limited capacity, which means that it can only hold a certain number of messages at a time.
/// If the channel is full, calls to [`Sender::send`] will block until there is space available.
///
/// Bounded channels are useful for controlling the flow of data between threads. For example, you can use a bounded
/// channel to prevent a producer thread from overwhelming a consumer thread.
///
/// Unlike an [`unbounded`] channel, if there is no space left for new messages, calls to
/// [`Sender::send`] will block (unblocking once a receiver has made space). If blocking behaviour
/// is not desired, [`Sender::try_send`] may be used.
///
/// Also 'rendezvous' channels are supported by loole. A bounded queue with a limited maximum capacity of zero will
/// block senders until a receiver is available to take the value.
///
/// Producers can send and consumers can receive messages asynchronously or synchronously.
///
/// # Examples
/// ```
/// let (tx, rx) = loole::bounded(3);
///
/// tx.send(1).unwrap();
/// tx.send(2).unwrap();
/// tx.send(3).unwrap();
///
/// assert!(tx.try_send(4).is_err());
///
/// let mut sum = 0;
/// sum += rx.recv().unwrap();
/// sum += rx.recv().unwrap();
/// sum += rx.recv().unwrap();
///
/// assert!(rx.try_recv().is_err());
///
/// assert_eq!(sum, 1 + 2 + 3);
/// ```
pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    channel(Some(cap))
}

/// Creates an unbounded channel, which has unlimited capacity.
///
/// This function creates a pair of sender and receiver halves of a channel. Values sent on the sender half will be
/// received on the receiver half, in the same order in which they were sent. The channel is thread-safe, and both
/// sender and receiver halves can be sent to or shared between threads as needed. Additionally, both sender and
/// receiver halves can be cloned.
///
/// Producers can send and consumers can receive messages asynchronously or synchronously.
///
/// # Examples
/// ```
/// let (tx, rx) = loole::unbounded();
///
/// tx.send(10).unwrap();
/// assert_eq!(rx.recv().unwrap(), 10);
/// ```
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    channel(None)
}
