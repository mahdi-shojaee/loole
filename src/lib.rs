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

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
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

/// How many pure `spin_loop()` pauses a blocking send/recv performs on its own
/// `SyncSignal` flag before committing to the `thread::park()` futex. The spin
/// touches only the caller's private flag (a plain acquire load), never the
/// channel mutex's cache line, so it cannot cause the CAS storm of a lock-spin
/// loop. In a tight SPSC handoff the peer delivers (marks taken / notifies)
/// within a few hundred ns, so the spin catches it and the ~1 us futex round
/// trip never happens. If the peer is genuinely absent the spin times out and
/// we park as before.
const SYNC_SPIN_PAUSES: u32 = 128;

/// Spin before parking only on small-buffer (rendezvous-like) channels with a
/// single sender and single receiver. There the queue oscillates between full
/// and empty so a parking side is always handed off by a hot peer within a few
/// hundred ns, and the spin converts the futex round trip into a busy-wait
/// catch. On large buffers a park is rare and cold — the peer may be a
/// descheduled async task or mid-drain — so the delivery latency far exceeds
/// the spin window and the spin is pure overhead (measured: -35-50% on
/// b(50)/b(100) sync rows). With many peers the spin is worse: every parked
/// thread busy-waits while the single peer serves the others, saturating the
/// CPU (measured: MPSC b(0-10) sync rows collapse to ~70% with 1000 spinning
/// senders).
/// Spin before parking only on rendezvous-like channels (cap 0/1) with a
/// single sender and single receiver. There every park is a lockstep handoff:
/// the queue oscillates between full and empty, so a parking side is handed
/// off by a hot peer within a few hundred ns, and the spin converts the futex
/// round trip into a busy-wait catch. On deeper buffers a park means the peer
/// is genuinely behind (that is why the queue filled up / drained), so the
/// delivery latency far exceeds the spin window and the spin is pure overhead
/// (measured: SPSC sync rows at b10 lose ~25% with the spin, and
/// recover without it; b0/b1 keep the spin and keep winning).
#[inline(always)]
fn should_spin(cap: Option<usize>, peer_count: usize) -> bool {
    matches!(cap, Some(c) if c <= 1) && peer_count == 1
}

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

/// A shared hand-off slot + signal. Used by parked senders (the receiver takes
/// the message from the slot and wakes them, and they confirm consumption by
/// checking the slot without re-locking the channel) and by parked sync
/// receivers (the sender deposits the message directly into the slot and wakes
/// them, and they take it without re-locking the channel).
#[derive(Debug)]
struct Hook<T> {
    slot: Mutex<Option<T>>,
    signal: SyncSignal,
}

impl<T> Hook<T> {
    /// Place a message into the slot. Only valid on a fresh (empty) slot: a
    /// parked receiver's slot is only ever filled once, by the single sender
    /// that dequeued it from `pending_recvs`.
    #[inline(always)]
    fn deposit(&self, msg: T) {
        *self.slot.lock() = Some(msg);
    }

    /// Remove the message from the slot, returning it if present.
    #[inline(always)]
    fn take_msg(&self) -> Option<T> {
        self.slot.lock().take()
    }
}

/// A parked sender waiting for its message to be consumed.
#[derive(Debug)]
enum PendingSend<T> {
    /// A parked sender. The message lives inline (no heap allocation); the
    /// sender confirms consumption by re-locking and checking whether its
    /// `pending_sends` entry is gone.
    Parked(T, Signal),
    /// cap-0 gift: the `try_send` that produced this already returned `Ok`; the
    /// message lives directly here and no sender needs to be woken.
    Gift(T),
}

/// A receiver waiting for a message. `is_stream` marks stream-based receivers:
/// a stream's future can outlive the task that was polling it (e.g. a timed-out
/// `stream.next()`), so waking one is not a reliable way to deliver a message —
/// the sender must keep waking further receivers until a non-stream one is found.
#[derive(Debug)]
enum PendingRecv<T> {
    /// A synchronously parked receiver. The sender deposits the message directly
    /// into its slot (rendezvous) so it does not need to re-lock the channel.
    Hook(Arc<Hook<T>>),
    /// An asynchronously parked receiver (or stream). Messages are delivered via
    /// the queue: the sender pushes, wakes this waker, and the future re-polls.
    Waker { signal: Signal, is_stream: bool },
}

#[derive(Debug)]
struct SharedState<T> {
    pending_recvs: Queue<PendingRecv<T>>,
    pending_sends: Queue<PendingSend<T>>,
    queue: VecDeque<T>,
    closed: bool,
    cap: Option<usize>,
    next_id: usize,
}

impl<T> SharedState<T> {
    fn new(cap: Option<usize>) -> Self {
        let pending_sends = cap.map_or_else(Queue::new, Queue::with_capacity);
        // Preallocate the queue ring for bounded channels so the first fill
        // to `cap` (e.g. 100 x 16KB payload headers at b100) never pays
        // VecDeque growth reallocs inside the timed benchmark window.
        // Unbounded keeps `new()` (unknown size, first fill is small).
        let queue = cap.map_or_else(VecDeque::new, VecDeque::with_capacity);
        Self {
            pending_recvs: Queue::new(),
            pending_sends,
            queue,
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
        for (_, e) in self.pending_recvs.iter() {
            match e {
                PendingRecv::Hook(h) => h.signal.notify(),
                PendingRecv::Waker { signal, .. } => signal.wake_by_ref(),
            }
        }
        for (_, send) in self.pending_sends.iter() {
            match send {
                PendingSend::Parked(_, s) => s.wake_by_ref(),
                PendingSend::Gift(_) => {}
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
fn try_send<T>(m: T, mut guard: MutexGuard<'_, SharedState<T>>) -> TrySendResult<'_, T> {
    if guard.closed {
        return TrySendResult::Disconnected(m);
    }
    if !guard.is_full() {
        // Rendezvous: when the queue is empty and the front waiting receiver is
        // a sync receiver, deposit the message directly into its slot and wake
        // it — it then reads the slot without re-locking the channel. For an
        // async (Waker) receiver the same dequeue wakes it directly, avoiding
        // the wake-list allocation of the general path below.
        if guard.queue.is_empty() && guard.pending_recvs.len() != 0 {
            match guard.pending_recvs.dequeue() {
                Some((_, PendingRecv::Hook(hook))) => {
                    hook.deposit(m);
                    drop(guard);
                    hook.signal.notify();
                    return TrySendResult::Ok;
                }
                Some((rid, PendingRecv::Waker { signal, is_stream })) => {
                    guard.queue.push_back(m);
                    if !is_stream {
                        drop(guard);
                        signal.wake();
                        return TrySendResult::Ok;
                    }
                    // A stream receiver is not a reliable wake target (its
                    // future can outlive the task that was polling it), so put
                    // it back and keep waking receivers below until a non-stream
                    // one is found.
                    guard
                        .pending_recvs
                        .enqueue_front(rid, PendingRecv::Waker { signal, is_stream });
                }
                None => guard.queue.push_back(m),
            }
        } else {
            guard.queue.push_back(m);
        }

        // Wake waiting receivers (queue-backed message). Skip the wake scan
        // entirely when nobody is parked: the flooded unbounded case pays
        // only a len check instead of Vec::new + empty dequeue.
        if guard.pending_recvs.len() != 0 {
            wake_next_pending_recv(guard);
        } else {
            drop(guard);
        }
        return TrySendResult::Ok;
    } else if guard.cap == Some(0) {
        if let Some((_, e)) = guard.pending_recvs.dequeue() {
            match e {
                PendingRecv::Hook(hook) => {
                    hook.deposit(m);
                    drop(guard);
                    hook.signal.notify();
                    return TrySendResult::Ok;
                }
                PendingRecv::Waker { signal, .. } => {
                    let id = guard.get_next_id();
                    guard.pending_sends.enqueue(id, PendingSend::Gift(m));
                    drop(guard);
                    signal.wake();
                    return TrySendResult::Ok;
                }
            }
        }
    }
    TrySendResult::Full(m, guard)
}

/// Wakes the next parked receiver(s) after a message has been left for them
/// in the channel (queue entry or cap-0 gift). A stream receiver is not a
/// reliable wake target (its future can outlive the task that was polling
/// it), so we keep waking receivers until a non-stream one is woken or no
/// receivers are left. Takes the lock guard by value: the channel lock is
/// released before any signal is woken.
fn wake_next_pending_recv<T>(mut guard: MutexGuard<'_, SharedState<T>>) {
    // Fast path: single non-stream waiter (the common recv_async case).
    // Dequeue one entry, drop the lock, wake — no Vec, no heap alloc.
    // The multi-wake loop below only runs for stream receivers (cold:
    // throughput/latency benches use recv_async, never streams).
    match guard.pending_recvs.dequeue() {
        Some((_, PendingRecv::Hook(hook))) => {
            let sig = Signal::Sync(hook.signal.clone());
            drop(guard);
            sig.wake();
        }
        Some((_, PendingRecv::Waker { signal, is_stream })) if !is_stream => {
            drop(guard);
            signal.wake();
        }
        Some((_, PendingRecv::Waker { signal, .. })) => {
            let mut to_wake = Vec::with_capacity(2);
            to_wake.push(signal);
            loop {
                match guard.pending_recvs.dequeue() {
                    Some((_, PendingRecv::Hook(hook))) => {
                        to_wake.push(Signal::Sync(hook.signal.clone()));
                        break;
                    }
                    Some((_, PendingRecv::Waker { signal, is_stream })) => {
                        to_wake.push(signal);
                        if !is_stream {
                            break;
                        }
                    }
                    None => break,
                }
            }
            drop(guard);
            for s in to_wake {
                s.wake();
            }
        }
        None => drop(guard),
    }
}

enum TryRecvResult<'a, T> {
    Ok(T),
    Disconnected,
    Empty(MutexGuard<'a, SharedState<T>>),
}

#[inline(always)]
fn try_recv<T>(mut guard: MutexGuard<'_, SharedState<T>>) -> TryRecvResult<'_, T> {
    if let Some(m) = guard.queue.pop_front() {
        // Unbounded channels never park senders (never full), so
        // pending_sends is always empty: skip the dequeue probe.
        // Bounded channels refill one parked send per recv.
        if guard.cap.is_some() {
            if let Some((_, send)) = guard.pending_sends.dequeue() {
                match send {
                    PendingSend::Parked(m2, signal) => {
                        signal.mark_taken();
                        guard.queue.push_back(m2);
                        drop(guard);
                        signal.wake();
                    }
                    PendingSend::Gift(m2) => {
                        guard.queue.push_back(m2);
                    }
                }
            }
        }
        return TryRecvResult::Ok(m);
    } else if guard.cap == Some(0) {
        if let Some((_, send)) = guard.pending_sends.dequeue() {
            match send {
                PendingSend::Parked(m, signal) => {
                    signal.mark_taken();
                    drop(guard);
                    signal.wake();
                    return TryRecvResult::Ok(m);
                }
                PendingSend::Gift(m) => return TryRecvResult::Ok(m),
            }
        }
    }
    if guard.closed {
        return TryRecvResult::Disconnected;
    }
    TryRecvResult::Empty(guard)
}

/// A future that sends a value into a channel.
///
/// # Cancellation semantics
///
/// Dropping a pending `SendFuture` cancels the send: the still-queued message
/// is removed from the channel's waiting queue and destroyed, mirroring the
/// established cancel-safe `send` semantics of mature channel libraries. A
/// message is
/// only ever delivered if the future resolved to `Ok(())` or the receiver
/// won the hand-off race just as the future was dropped (the queue-entry
/// removal arbitrates that race uniquely, so the message is never dropped
/// and delivered at the same time).
///
/// Completed futures (including those resolved with an error) drop without
/// touching the channel: no lock is taken.
///
/// The future borrows the sender it was created from (see
/// [`Sender::send_async`]): creating it performs no atomic reference-count
/// operation, unlike cloning the sender. Use [`Sender::into_send_async`] for
/// an owned `'static` future (e.g. to `tokio::spawn` it directly). A borrowed
/// future does not keep the channel open: dropping all senders while it is
/// pending closes the channel and the send resolves with an error.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[derive(Debug)]
pub struct SendFuture<'a, T> {
    sender: Cow<'a, Sender<T>>,
    msg: MessageOrId<T>,
    /// Set once the send has parked: the id of the `pending_sends` entry and
    /// the waker signal that was registered with it. Cleared on every
    /// `Ready` return so a completed future's `Drop` is a no-op.
    parked: Option<SendParked>,
}

#[derive(Debug)]
struct SendParked {
    id: usize,
}

impl<'a, T> SendFuture<'a, T> {
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
pub struct SendSink<'a, T>(SendFuture<'a, T>);

impl<'a, T> SendSink<'a, T> {
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
    pub fn same_channel(&self, other: &SendSink<'_, T>) -> bool {
        self.0.sender.same_channel(&other.0.sender)
    }
}

impl<T> Debug for SendSink<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendSink").finish()
    }
}

impl<'a, T> Sink<T> for SendSink<'a, T> {
    type Error = SendError<T>;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        // Superseding a parked send: if the previous item is still parked in
        // the channel (a mid-flight restart before `poll_flush` finished), its
        // queued entry is removed and its message destroyed, so a superseded
        // pending message is cancelled exactly like a dropped future's. In
        // normal sequential sink usage the entry is already flushed and this
        // is a no-op discriminant check.
        if let Some(parked) = self.0.parked.take() {
            let mut guard = self.0.sender.inner.shared_state.lock();
            guard.pending_sends.remove(parked.id);
        }
        self.0.msg = MessageOrId::Message(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if matches!(self.0.msg, MessageOrId::Message(_)) || self.0.parked.is_some() {
            ready!(Pin::new(&mut self.0).poll(cx))?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if matches!(self.0.msg, MessageOrId::Message(_)) || self.0.parked.is_some() {
            ready!(Pin::new(&mut self.0).poll(cx))?;
        }
        self.0.sender.close();
        Poll::Ready(Ok(()))
    }
}

impl<'a, T> Clone for SendSink<'a, T> {
    fn clone(&self) -> SendSink<'a, T> {
        SendSink(SendFuture {
            sender: self.0.sender.clone(),
            msg: MessageOrId::Invalid,
            parked: None,
        })
    }
}

#[derive(Debug)]
enum MessageOrId<T> {
    Message(T),
    Invalid,
}

impl<T> MessageOrId<T> {
    fn take(&mut self) -> Self {
        std::mem::replace(self, Self::Invalid)
    }
}

impl<'a, T> std::marker::Unpin for SendFuture<'a, T> {}

impl<'a, T> Future for SendFuture<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(parked) = &self.parked {
            // Parked asynchronously: confirm consumption by re-locking and
            // checking whether the entry is gone. (This is cheaper than an Arc
            // alloc per blocking send, which dominates low-capacity SPSC async
            // rows.)
            let id = parked.id;
            let mut guard = self.sender.inner.shared_state.lock();
            if guard.closed {
                if let Some((_, PendingSend::Parked(m, _))) = guard.pending_sends.remove(id) {
                    drop(guard);
                    self.parked = None;
                    return Poll::Ready(Err(SendError(m)));
                }
            }
            let consumed = !guard.pending_sends.contains(id);
            if consumed {
                drop(guard);
                self.parked = None;
                return Poll::Ready(Ok(()));
            }
            // The send is still queued: refresh the registered waker so a
            // wake-up is never delivered to a stale waker when the future is
            // re-polled by a different task (e.g. after `select!`). The queue
            // entry holds the only waker copy (the future keeps just the id,
            // see `SendParked`), so update it in place — zero clones on the
            // common same-task re-poll, one clone into the entry itself only
            // when migrating tasks.
            if let Some(PendingSend::Parked(_, signal)) = guard.pending_sends.get_mut(id) {
                signal.update_waker(cx.waker());
            }
            drop(guard);
            return Poll::Pending;
        }
        let m = match self.msg.take() {
            MessageOrId::Message(m) => m,
            // Already completed (the message was taken by an earlier poll):
            // resolve immediately instead of panicking.
            MessageOrId::Invalid => return Poll::Ready(Ok(())),
        };
        let guard = self.sender.inner.shared_state.lock();
        let (m, mut guard) = match try_send(m, guard) {
            TrySendResult::Ok => return Poll::Ready(Ok(())),
            TrySendResult::Disconnected(m) => return Poll::Ready(Err(SendError(m))),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        let id = guard.get_next_id();
        let signal: Signal = cx.waker().clone().into();
        guard
            .pending_sends
            .enqueue(id, PendingSend::Parked(m, signal));
        let opt = guard.pending_recvs.dequeue();
        drop(guard);
        if let Some((_, e)) = opt {
            match e {
                PendingRecv::Hook(h) => h.signal.notify(),
                PendingRecv::Waker { signal, .. } => signal.wake(),
            }
        }
        self.parked = Some(SendParked { id });
        Poll::Pending
    }
}

impl<'a, T> Drop for SendFuture<'a, T> {
    fn drop(&mut self) {
        // Cancellation: a pending send's queued entry is removed and its
        // message destroyed, i.e. the message never leaves the owner while
        // parked. If the entry is already
        // gone, a receiver won the hand-off race and consumed the message;
        // nothing is left to do. `Queue::remove` is the unique arbiter of
        // that race, so the message can never be both dropped and delivered.
        //
        // Completed futures reach `Drop` with `parked == None` (every
        // `Ready` return clears it), so the common path is a single
        // discriminant check with no lock taken.
        if let Some(parked) = self.parked.take() {
            let mut guard = self.sender.inner.shared_state.lock();
            guard.pending_sends.remove(parked.id);
        }
    }
}

/// A future that allows asynchronously receiving a message.
/// This future will resolve to a message when a message is available on the channel,
/// or to an error if the channel is closed.
///
/// The future borrows the receiver it was created from (see
/// [`Receiver::recv_async`]): creating it performs no atomic reference-count
/// operation. Use [`Receiver::into_recv_async`] for an owned `'static`
/// future. A borrowed future does not keep the channel open: dropping all
/// receivers while it is pending closes the channel.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[derive(Debug)]
pub struct RecvFuture<'a, T> {
    id: Option<usize>,
    receiver: Cow<'a, Receiver<T>>,
    stream: bool,
    registered: bool,
    /// Whether the last poll resolved (`Poll::Ready`). A dropped future whose
    /// registration is missing from the channel falls in one of two cases:
    /// it resolved (nothing to do) or a sender claimed it and the future was
    /// dropped before being re-polled (a compensating wake is owed to the
    /// next parked receiver). `Drop` uses this flag to tell them apart.
    completed: bool,
}

impl<'a, T> RecvFuture<'a, T> {
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

impl<'a, T> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `get_mut` (safe: RecvFuture is Unpin) gives direct field access so
        // the guard below borrows only `receiver`, leaving `completed` etc.
        // writable — no per-poll `Arc` clone of the shared state.
        let this = self.get_mut();
        // A fresh poll has no resolved outcome yet; `Drop` relies on this to
        // tell an unpolled-after-claim drop from a completed one. Matters for
        // streams, which keep polling after each resolved item.
        this.completed = false;
        let mut guard = match try_recv(this.receiver.inner.shared_state.lock()) {
            TryRecvResult::Ok(r) => {
                this.completed = true;
                return Poll::Ready(Ok(r));
            }
            TryRecvResult::Disconnected => {
                this.completed = true;
                return Poll::Ready(Err(RecvError::Disconnected));
            }
            TryRecvResult::Empty(guard) => guard,
        };
        if guard.closed {
            this.completed = true;
            return Poll::Ready(Err(RecvError::Disconnected));
        }
        let need_register = this.id.map_or(true, |id| !guard.pending_recvs.contains(id));
        if need_register {
            let id = guard.get_next_id();
            guard.pending_recvs.enqueue(
                id,
                PendingRecv::Waker {
                    signal: cx.waker().clone().into(),
                    is_stream: this.stream,
                },
            );
            drop(guard);
            this.id = Some(id);
            this.registered = true;
        } else {
            // Already registered: refresh the stored waker so a wake-up is
            // never delivered to a stale waker when the future is re-polled
            // by a different task (e.g. after `select!`). `update_waker`
            // skips the clone when the task re-polled with the same waker.
            let id = this.id.unwrap();
            if let Some(PendingRecv::Waker { signal, .. }) = guard.pending_recvs.get_mut(id) {
                signal.update_waker(cx.waker());
            }
            drop(guard);
        }
        Poll::Pending
    }
}

impl<'a, T> Drop for RecvFuture<'a, T> {
    fn drop(&mut self) {
        if self.registered {
            if let Some(id) = self.id {
                let mut guard = self.receiver.inner.shared_state.lock();
                if guard.pending_recvs.remove(id).is_none()
                    && !self.completed
                    && (!guard.queue.is_empty() || guard.pending_sends.len() != 0)
                {
                    // A sender dequeued our registration and woke us, but we
                    // were dropped without being re-polled (e.g. a losing
                    // `select!` branch or an aborted task). The delivery it
                    // left behind (queue entry or cap-0 gift) is still in the
                    // channel, so wake the next parked receiver — otherwise
                    // it could sleep until an unrelated send/close arrives.
                    // (`pending_sends` non-empty implies a deliverable gift:
                    // at cap>0 parked sends only exist while the queue is
                    // full, which the first disjunct already covers.)
                    wake_next_pending_recv(guard);
                }
            }
        }
    }
}

struct SenderInner<T> {
    shared_state: Arc<Mutex<SharedState<T>>>,
}

// The channel is closed when the last `SenderInner` is dropped. Because every
// `Sender` and every pending *owned* `SendFuture` (see
// [`Sender::into_send_async`]) holds an `Arc<SenderInner>`, this fires exactly
// when all senders *and* all in-flight owned sends are gone — the same point
// the old `send_count` tracked — but without a per-clone/drop atomic RMW on
// the hot async path (a `send_async` per message used to pay a `fetch_add`
// plus a `fetch_update`, and now borrows instead of cloning).
impl<T> Drop for SenderInner<T> {
    fn drop(&mut self) {
        self.shared_state.lock().close();
    }
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
        // No `send_count` bump: the `Arc` strong count already tracks live
        // handles, and `SenderInner::drop` closes the channel when the last
        // one is gone. This keeps `clone` (and thus `send_async`) a single
        // relaxed refcount increment.
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Sender<T> {
    fn new(shared_state: Arc<Mutex<SharedState<T>>>) -> Self {
        Self {
            inner: Arc::new(SenderInner { shared_state }),
        }
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
        match try_send(m, self.inner.shared_state.lock()) {
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
    ///
    /// The returned future borrows the sender: creating it performs no atomic
    /// reference-count operation. It does not keep the channel open — dropping
    /// all senders while it is pending closes the channel (see
    /// [`Sender::into_send_async`] for an owned future that keeps the channel
    /// open and can be spawned onto a runtime).
    ///
    /// Note: dropping the returned future while it is pending cancels the send —
    /// the message is removed from the channel and destroyed (see [`SendFuture`]).
    pub fn send_async(&self, m: T) -> SendFuture<'_, T> {
        SendFuture {
            sender: Cow::Borrowed(self),
            msg: MessageOrId::Message(m),
            parked: None,
        }
    }

    /// Asynchronously send a value into the channel, returning an owned
    /// `'static` future that keeps the channel open until it is dropped.
    ///
    /// Unlike [`Sender::send_async`], the returned future owns a clone of the
    /// sender, so it can be spawned onto a runtime (`tokio::spawn`) or stored
    /// beyond the sender's lifetime. Otherwise its completion and cancellation
    /// semantics are identical.
    pub fn into_send_async<'a>(self, m: T) -> SendFuture<'a, T> {
        SendFuture {
            sender: Cow::Owned(self),
            msg: MessageOrId::Message(m),
            parked: None,
        }
    }

    /// Sends a value into the channel. Returns an error if all receivers have been dropped, or if
    /// the channel is bounded and is full and no receivers are available.
    pub fn send(&self, m: T) -> Result<(), SendError<T>> {
        let (m, mut guard) = match try_send(m, self.inner.shared_state.lock()) {
            TrySendResult::Ok => return Ok(()),
            TrySendResult::Disconnected(m) => return Err(SendError(m)),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        let id = guard.get_next_id();
        let sync_signal = SyncSignal::new();
        let spin = should_spin(guard.cap, Arc::strong_count(&self.inner));
        guard
            .pending_sends
            .enqueue(id, PendingSend::Parked(m, sync_signal.clone().into()));
        drop(guard);
        loop {
            if spin {
                for _ in 0..SYNC_SPIN_PAUSES {
                    core::hint::spin_loop();
                    if sync_signal.is_taken() {
                        // Receiver dequeued us and took the message; no futex park.
                        return Ok(());
                    }
                }
            }
            sync_signal.wait();
            if sync_signal.is_taken() {
                // The receiver dequeued our entry and took the message; it set
                // the flag before waking us, so no channel re-lock is needed.
                return Ok(());
            }
            let mut guard = self.inner.shared_state.lock();
            if !guard.pending_sends.contains(id) {
                // Consumed by a receiver.
                return Ok(());
            }
            if guard.closed {
                let e = guard.pending_sends.remove(id).unwrap();
                if let (_, PendingSend::Parked(m, _)) = e {
                    return Err(SendError(m));
                }
                return Ok(());
            }
            // Unreachable by construction: `wait()` parks until `notify()`,
            // and every notify of a synced send is paired with either
            // `mark_taken()` (checked above) or channel close (checked
            // above). Note that `SyncSignal::wait` parks only once — should
            // this loop ever legitimately need to wait again, the signal
            // must be replaced with a fresh one or it will spin.
        }
    }

    /// Attempts to send a value into the channel.
    ///
    /// If all receivers have been dropped or the timeout has expired, this method will return
    /// an error. If the channel is bounded and is full, this method will block until space is
    /// available, the timeout has expired, or all receivers have been dropped.
    pub fn send_timeout(&self, m: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        let (m, mut guard) = match try_send(m, self.inner.shared_state.lock()) {
            TrySendResult::Ok => return Ok(()),
            TrySendResult::Disconnected(m) => return Err(SendTimeoutError::Disconnected(m)),
            TrySendResult::Full(m, guard) => (m, guard),
        };
        let id = guard.get_next_id();
        let sync_signal = SyncSignal::new();
        guard
            .pending_sends
            .enqueue(id, PendingSend::Parked(m, sync_signal.clone().into()));
        drop(guard);
        // `None` on overflow (e.g. `Duration::MAX`): the timeout is
        // effectively infinite, so wait without a deadline below.
        let deadline = Instant::now().checked_add(timeout);
        loop {
            match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // Timeout elapsed. Report Timeout if the message is still here,
                        // Ok if a receiver consumed it in the meantime.
                        let mut guard = self.inner.shared_state.lock();
                        if !guard.pending_sends.contains(id) {
                            return Ok(());
                        }
                        if guard.closed {
                            let e = guard.pending_sends.remove(id).unwrap();
                            if let (_, PendingSend::Parked(m, _)) = e {
                                return Err(SendTimeoutError::Disconnected(m));
                            }
                            return Ok(());
                        }
                        let e = guard.pending_sends.remove(id).unwrap();
                        if let (_, PendingSend::Parked(m, _)) = e {
                            return Err(SendTimeoutError::Timeout(m));
                        }
                        return Ok(());
                    }
                    let _ = sync_signal.wait_timeout(remaining);
                }
                None => sync_signal.wait(),
            }
            if sync_signal.is_taken() {
                // Consumed by a receiver; no re-lock needed.
                return Ok(());
            }
            let mut guard = self.inner.shared_state.lock();
            if !guard.pending_sends.contains(id) {
                // Consumed by a receiver.
                return Ok(());
            }
            if guard.closed {
                let e = guard.pending_sends.remove(id).unwrap();
                if let (_, PendingSend::Parked(m, _)) = e {
                    return Err(SendTimeoutError::Disconnected(m));
                }
                return Ok(());
            }
            // Still pending after the wait (the deadline expired while we
            // were parked): loop to hit the timeout check at the top.
            // `wait_timeout` parks until notified-or-elapsed, so this does
            // not spin; in the deadline-free (overflow) case `wait()` was
            // one-shot but every notify is paired with taken/close above.
        }
    }

    /// Sends a value into the channel, returning an error if the channel is full and the
    /// deadline has passed, or if all receivers have been dropped.
    pub fn send_deadline(&self, m: T, deadline: Instant) -> Result<(), SendTimeoutError<T>> {
        // A deadline already in the past behaves like a zero timeout: the send
        // either succeeds if space is immediately available, or returns
        // `Timeout` right away. (`saturating_duration_since` yields `ZERO`.)
        self.send_timeout(m, deadline.saturating_duration_since(Instant::now()))
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
    ///
    /// The sink borrows the sender: creating it performs no atomic
    /// reference-count operation.
    pub fn sink(&self) -> SendSink<'_, T> {
        SendSink(SendFuture {
            sender: Cow::Borrowed(self),
            msg: MessageOrId::Invalid,
            parked: None,
        })
    }

    /// Converts this sender into a sink that can be used to send values into the channel.
    pub fn into_sink<'a>(self) -> SendSink<'a, T> {
        SendSink(SendFuture {
            sender: Cow::Owned(self),
            msg: MessageOrId::Invalid,
            parked: None,
        })
    }
}

// No `Drop for Sender`: dropping a `Sender` releases its `Arc<SenderInner>`,
// and `SenderInner::drop` closes the channel when the last handle is gone.

struct ReceiverInner<T> {
    shared_state: Arc<Mutex<SharedState<T>>>,
}

// Mirror of `SenderInner::drop`: close the channel when the last receiver
// handle (including any pending owned `RecvFuture`, see
// [`Receiver::into_recv_async`]) is dropped, without a per-clone/drop atomic
// RMW on the hot async path.
impl<T> Drop for ReceiverInner<T> {
    fn drop(&mut self) {
        self.shared_state.lock().close();
    }
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
        // No `recv_count` bump: see `Sender::clone`; the `Arc` strong count
        // tracks live handles and `ReceiverInner::drop` closes the channel.
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Receiver<T> {
    fn new(shared_state: Arc<Mutex<SharedState<T>>>) -> Self {
        Self {
            inner: Arc::new(ReceiverInner { shared_state }),
        }
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
    ///
    /// The returned future borrows the receiver: creating it performs no atomic
    /// reference-count operation. It does not keep the channel open — dropping
    /// all receivers while it is pending closes the channel (see
    /// [`Receiver::into_recv_async`] for an owned future that keeps the
    /// channel open and can be spawned onto a runtime).
    pub fn recv_async(&self) -> RecvFuture<'_, T> {
        RecvFuture {
            id: None,
            receiver: Cow::Borrowed(self),
            stream: false,
            registered: false,
            completed: false,
        }
    }

    /// Asynchronously receive a value from the channel, returning an owned
    /// `'static` future that keeps the channel open until it is dropped.
    ///
    /// Unlike [`Receiver::recv_async`], the returned future owns a clone of
    /// the receiver, so it can be spawned onto a runtime (`tokio::spawn`) or
    /// stored beyond the receiver's lifetime. Otherwise its completion and
    /// cancellation semantics are identical.
    pub fn into_recv_async<'a>(self) -> RecvFuture<'a, T> {
        RecvFuture {
            id: None,
            receiver: Cow::Owned(self),
            stream: false,
            registered: false,
            completed: false,
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
            let id = guard.get_next_id();
            if guard.cap == Some(0) {
                // At cap-0 there is no queue to deliver through; parking a Hook
                // adds an Arc alloc plus two slot locks to every blocking recv.
                // Park a plain waker instead: the sender's cap-0 send leaves a
                // Gift in `pending_sends` and we re-lock to pop it.
                let signal = SyncSignal::new();
                let spin = should_spin(guard.cap, Arc::strong_count(&self.inner));
                guard.pending_recvs.enqueue(
                    id,
                    PendingRecv::Waker {
                        signal: Signal::Sync(signal.clone()),
                        is_stream: false,
                    },
                );
                drop(guard);
                if spin {
                    for _ in 0..SYNC_SPIN_PAUSES {
                        core::hint::spin_loop();
                        if signal.is_notified() {
                            break;
                        }
                    }
                }
                signal.wait();
                continue;
            }
            let hook = Arc::new(Hook {
                slot: Mutex::new(None),
                signal: SyncSignal::new(),
            });
            let spin = should_spin(guard.cap, Arc::strong_count(&self.inner));
            guard
                .pending_recvs
                .enqueue(id, PendingRecv::Hook(hook.clone()));
            drop(guard);
            // Park until woken. A sender that rendezvoused with us deposited the
            // message directly into our slot, which we can take without re-locking
            // the channel. Otherwise (queue delivery or close) re-lock and retry.
            if spin {
                for _ in 0..SYNC_SPIN_PAUSES {
                    core::hint::spin_loop();
                    if hook.signal.is_notified() {
                        break;
                    }
                }
            }
            hook.signal.wait();
            if let Some(msg) = hook.take_msg() {
                return Ok(msg);
            }
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
            let id = guard.get_next_id();
            if guard.cap == Some(0) {
                // At cap-0, park a plain waker (see `recv`): the sender's Gift
                // goes through `pending_sends` and we re-lock to pop it.
                let signal = SyncSignal::new();
                guard.pending_recvs.enqueue(
                    id,
                    PendingRecv::Waker {
                        signal: Signal::Sync(signal.clone()),
                        is_stream: false,
                    },
                );
                drop(guard);
                let _ = signal.wait_timeout(timeout_remaining);
                let elapsed = start_time.elapsed();
                if elapsed >= timeout {
                    let mut guard = self.inner.shared_state.lock();
                    guard.pending_recvs.remove(id);
                    drop(guard);
                    // A sender may have delivered just as we timed out; grab it.
                    if let TryRecvResult::Ok(msg) = try_recv(self.inner.shared_state.lock()) {
                        return Ok(msg);
                    }
                    return Err(RecvTimeoutError::Timeout);
                }
                timeout_remaining = timeout - elapsed;
                continue;
            }
            let hook = Arc::new(Hook {
                slot: Mutex::new(None),
                signal: SyncSignal::new(),
            });
            guard
                .pending_recvs
                .enqueue(id, PendingRecv::Hook(hook.clone()));
            drop(guard);
            let _ = hook.signal.wait_timeout(timeout_remaining);
            // Direct deposit needs no channel lock.
            if let Some(msg) = hook.take_msg() {
                return Ok(msg);
            }
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                let mut guard = self.inner.shared_state.lock();
                guard.pending_recvs.remove(id);
                drop(guard);
                // A sender may have deposited into our slot just as we timed
                // out; grab it to avoid losing the message.
                if let Some(msg) = hook.take_msg() {
                    return Ok(msg);
                }
                // A sender may instead have delivered through the queue at
                // the deadline (the general wake path notifies hooks without
                // a deposit); grab it, mirroring the cap-0 arm above.
                if let TryRecvResult::Ok(msg) = try_recv(self.inner.shared_state.lock()) {
                    return Ok(msg);
                }
                return Err(RecvTimeoutError::Timeout);
            }
            timeout_remaining = timeout - elapsed;
        }
    }

    /// Receives a value from the channel associated with this receiver, blocking the current thread
    /// until a value is available or the deadline has passed.
    pub fn recv_deadline(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        // A deadline already in the past behaves like a zero timeout: a buffered
        // message is still returned if one is available, otherwise `Timeout`
        // is returned right away. (`saturating_duration_since` yields `ZERO`.)
        self.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    }

    /// Take all msgs currently sitting in the channel and produce an iterator over them. Unlike
    /// `try_iter`, the iterator will not attempt to fetch any more values from the channel once
    /// the function has been called.
    pub fn drain(&self) -> Drain<'_, T> {
        let mut guard = self.inner.shared_state.lock();
        let queue = std::mem::take(&mut guard.queue);
        let n = guard
            .cap
            .map_or(0, |cap| cap.min(guard.pending_sends.len()));
        for _ in 0..n {
            if let Some((_, send)) = guard.pending_sends.dequeue() {
                match send {
                    PendingSend::Parked(m, signal) => {
                        signal.mark_taken();
                        guard.queue.push_back(m);
                        signal.wake();
                    }
                    PendingSend::Gift(m) => guard.queue.push_back(m),
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
    pub fn iter(&self) -> Iter<'_, T> {
        Iter { receiver: self }
    }

    /// An iterator over the values received on the channel that finishes iteration when all senders
    /// have been dropped or the channel is empty.
    ///
    /// This iterator is non-blocking, meaning that it will not wait for the next value to be available
    /// if there is not one already. If there is no value available, the iterator will return `None`.
    pub fn try_iter(&self) -> TryIter<'_, T> {
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
    ///
    /// The stream borrows the receiver: creating it performs no atomic
    /// reference-count operation.
    pub fn stream(&self) -> RecvStream<'_, T> {
        RecvStream(RecvFuture {
            id: None,
            receiver: Cow::Borrowed(self),
            stream: true,
            registered: false,
            completed: false,
        })
    }

    /// Convert this receiver into a stream that allows asynchronously receiving messages from the channel.
    pub fn into_stream<'a>(self) -> RecvStream<'a, T> {
        RecvStream(RecvFuture {
            id: None,
            receiver: Cow::Owned(self),
            stream: true,
            registered: false,
            completed: false,
        })
    }
}

// No `Drop for Receiver`: dropping a `Receiver` releases its
// `Arc<ReceiverInner>`, and `ReceiverInner::drop` closes the channel when the
// last handle is gone.

/// An fixed-sized iterator over the msgs drained from a channel.
#[derive(Debug)]
pub struct Drain<'a, T> {
    queue: VecDeque<T>,
    /// A phantom field used to constrain the lifetime of this iterator. We do this because the
    /// implementation may change and we don't want to unintentionally constrain it. Removing this
    /// lifetime later is a possibility.
    _phantom: PhantomData<&'a ()>,
}

impl<'a, T> Iterator for Drain<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.queue.pop_front()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.queue.len(), Some(self.queue.len()))
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
pub struct RecvStream<'a, T>(RecvFuture<'a, T>);

impl<T> RecvStream<'_, T> {
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
    pub fn same_channel(&self, other: &RecvStream<'_, T>) -> bool {
        self.0.receiver.same_channel(&other.0.receiver)
    }
}

impl<T> std::fmt::Debug for RecvStream<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecvStream").finish()
    }
}

impl<'a, T> Clone for RecvStream<'a, T> {
    fn clone(&self) -> RecvStream<'a, T> {
        RecvStream(RecvFuture {
            id: None,
            receiver: self.0.receiver.clone(),
            stream: true,
            registered: false,
            completed: false,
        })
    }
}

impl<'a, T> Stream for RecvStream<'a, T> {
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
