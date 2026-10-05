//! Adversarial inline-wake executor: a waker that runs arbitrary code the
//! moment it is woken — here, re-entering the channel through the public API
//! (`len()` takes the channel lock). If the channel ever wakes a waiter
//! while holding its lock, such a waker self-deadlocks the waking thread.
//!
//! `close()` and `drain()` are the two sites that historically woke under
//! the lock; these tests pin the release-before-wake discipline that
//! `try_recv`/`wake_next_pending_recv` already follow.
//!
//! Structure note: every channel handle (including the parked future and
//! the probing waker) lives inside the victim thread, and the main thread
//! holds only an mpsc receiver. Under a wake-under-lock regression the
//! victim thread wedges holding the channel lock, so dropping any channel
//! handle from the main thread would hang on that lock during unwind —
//! this construction keeps the failure a clean watchdog timeout instead.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    task::{Context, Poll, Wake, Waker},
    thread,
    time::Duration,
};

use loole::{bounded, Receiver, RecvError, Sender};

/// Watchdog budget after which a wedged operation is declared a deadlock.
/// A healthy release-before-wake completes in microseconds; this leaves
/// headroom for a heavily loaded CI machine.
const WATCHDOG: Duration = Duration::from_secs(2);

struct SenderProbe {
    tx: Sender<u8>,
    woken: AtomicBool,
}

impl Wake for SenderProbe {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        // Re-enter the channel: takes the channel lock.
        let _ = self.tx.len();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        let _ = self.tx.len();
    }
}

struct ReceiverProbe {
    rx: Receiver<u8>,
    woken: AtomicBool,
}

impl Wake for ReceiverProbe {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        let _ = self.rx.len();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        let _ = self.rx.len();
    }
}

#[test]
fn close_wakes_only_after_releasing_the_channel_lock() {
    // (closed, resolved_disconnected, woken)
    let (done_tx, done_rx) = mpsc::channel::<Result<(bool, bool, bool), &'static str>>();
    thread::spawn(move || {
        let (tx, rx) = bounded::<u8>(1);
        let probe = Arc::new(ReceiverProbe {
            rx: rx.clone(),
            woken: AtomicBool::new(false),
        });
        let waker = Waker::from(probe.clone());
        let cx = &mut Context::from_waker(&waker);

        // Park a receiver whose waker re-enters the channel the moment it
        // runs; then close. Under wake-under-lock the adversarial waker
        // deadlocks this thread on the channel mutex close() still holds.
        let mut f = rx.recv_async();
        if !Pin::new(&mut f).poll(cx).is_pending() {
            let _ = done_tx.send(Err("recv_async did not park"));
            return;
        }
        let closed = tx.close();
        let resolved = Pin::new(&mut f).poll(cx) == Poll::Ready(Err(RecvError::Disconnected));
        let woken = probe.woken.load(Ordering::SeqCst);
        let _ = done_tx.send(Ok((closed, resolved, woken)));
    });
    match done_rx.recv_timeout(WATCHDOG) {
        Ok(Ok((true, true, true))) => {}
        Ok(Ok((closed, resolved, woken))) => {
            panic!("postcondition violated: closed={closed} resolved={resolved} woken={woken}")
        }
        Ok(Err(msg)) => panic!("{msg}"),
        Err(_) => panic!("close() deadlocked: a waker ran while the channel lock was held"),
    }
}

#[test]
fn drain_wakes_only_after_releasing_the_channel_lock() {
    // (drained, send_resolved, queued_message_received, woken)
    let (done_tx, done_rx) = mpsc::channel::<Result<(Vec<u8>, bool, bool, bool), &'static str>>();
    thread::spawn(move || {
        let (tx, rx) = bounded::<u8>(1);
        tx.send(1).unwrap();
        let probe = Arc::new(SenderProbe {
            tx: tx.clone(),
            woken: AtomicBool::new(false),
        });
        let waker = Waker::from(probe.clone());
        let cx = &mut Context::from_waker(&waker);

        // Park a sender on the full channel; drain pulls and wakes it.
        let mut f = tx.send_async(2);
        if !Pin::new(&mut f).poll(cx).is_pending() {
            let _ = done_tx.send(Err("send_async did not park"));
            return;
        }
        let drained: Vec<u8> = rx.drain().collect();
        let send_resolved = Pin::new(&mut f).poll(cx) == Poll::Ready(Ok(()));
        let queued = rx.try_recv() == Ok(2);
        let woken = probe.woken.load(Ordering::SeqCst);
        let _ = done_tx.send(Ok((drained, send_resolved, queued, woken)));
    });
    match done_rx.recv_timeout(WATCHDOG) {
        Ok(Ok((drained, true, true, true))) => {
            assert_eq!(drained, [1], "drain returns the queued message");
        }
        Ok(Ok((drained, send_resolved, queued, woken))) => panic!(
            "postcondition violated: drained={drained:?} send_resolved={send_resolved} queued={queued} woken={woken}"
        ),
        Ok(Err(msg)) => panic!("{msg}"),
        Err(_) => panic!("drain() deadlocked: a waker ran while the channel lock was held"),
    }
}
