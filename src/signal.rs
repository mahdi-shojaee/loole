use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Waker,
    thread::{self, Thread},
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq)]
pub struct WaitTimeoutError;

impl std::error::Error for WaitTimeoutError {}

impl std::fmt::Display for WaitTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timeout")
    }
}

#[derive(Debug)]
pub struct AsyncSignal {
    waker: Waker,
}

impl Clone for AsyncSignal {
    fn clone(&self) -> Self {
        AsyncSignal {
            waker: self.waker.clone(),
        }
    }
}

impl AsyncSignal {
    /// Replace the stored waker with `waker` if they differ. Returns `true`
    /// when the stored waker was replaced, `false` when `will_wake` said the
    /// new waker is equivalent (common: same task re-polled).
    ///
    /// This is what makes re-polling a pending future with a *different*
    /// waker safe: the future keeps its queue slot but the slot now points at
    /// the new waker, so a wakeup is never delivered to a stale one.
    #[inline(always)]
    pub fn update_waker(&mut self, waker: &Waker) -> bool {
        if self.waker.will_wake(waker) {
            return false;
        }
        self.waker = waker.clone();
        true
    }
}

#[derive(Debug)]
struct SyncSignalInner {
    thread: Thread,
    notified: AtomicBool,
    park_called: AtomicBool,
    /// Set by the consumer of a parked sender just before waking it, so the
    /// sender can confirm consumption with an atomic load instead of a
    /// channel re-lock (see `Sender::send`).
    taken: AtomicBool,
}

#[derive(Debug)]
pub struct SyncSignal {
    inner: Arc<SyncSignalInner>,
}

#[derive(Debug)]
pub enum Signal {
    Async(AsyncSignal),
    Sync(SyncSignal),
}

impl From<Waker> for Signal {
    #[inline(always)]
    fn from(waker: Waker) -> Self {
        Self::Async(AsyncSignal { waker })
    }
}

impl From<AsyncSignal> for Signal {
    #[inline(always)]
    fn from(s: AsyncSignal) -> Self {
        Self::Async(s)
    }
}

impl SyncSignal {
    #[inline(always)]
    pub fn new() -> Self {
        SyncSignal {
            inner: Arc::new(SyncSignalInner {
                thread: thread::current(),
                notified: AtomicBool::new(false),
                park_called: AtomicBool::new(false),
                taken: AtomicBool::new(false),
            }),
        }
    }

    #[inline(always)]
    pub fn mark_taken(&self) {
        self.inner.taken.store(true, Ordering::Release);
    }

    #[inline(always)]
    pub fn is_taken(&self) -> bool {
        self.inner.taken.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn is_notified(&self) -> bool {
        self.notified()
    }

    #[inline(always)]
    fn notified(&self) -> bool {
        self.inner.notified.load(Ordering::Acquire)
    }

    #[inline(always)]
    fn park(&self) {
        while !self.notified() {
            thread::park();
        }
    }

    #[inline(always)]
    fn park_timeout(&self, timeout: Duration) -> Result<(), WaitTimeoutError> {
        let start_time = Instant::now();
        let mut remaining = timeout;
        loop {
            thread::park_timeout(remaining);
            if self.notified() {
                return Ok(());
            }
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                return Err(WaitTimeoutError);
            }
            remaining = timeout - elapsed;
        }
    }

    #[inline(always)]
    pub fn wait(&self) {
        if !self.inner.park_called.swap(true, Ordering::Relaxed) {
            self.park();
        }
    }

    #[inline(always)]
    pub fn wait_timeout(&self, timeout: Duration) -> Result<(), WaitTimeoutError> {
        if !self.inner.park_called.swap(true, Ordering::Relaxed) {
            return self.park_timeout(timeout);
        }
        Ok(())
    }

    #[inline(always)]
    pub fn notify(&self) {
        self.inner.notified.store(true, Ordering::Release);
        if self.inner.park_called.swap(true, Ordering::Relaxed) {
            self.inner.thread.unpark();
        }
    }
}

impl Clone for SyncSignal {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl From<SyncSignal> for Signal {
    #[inline(always)]
    fn from(s: SyncSignal) -> Self {
        Self::Sync(s)
    }
}

impl Signal {
    /// Mark the sender this signal belongs to as consumed. Only meaningful for
    /// sync signals: the receiver dequeues a parked sender's message and sets
    /// this before waking it so the sender can confirm without a re-lock.
    #[inline(always)]
    pub fn mark_taken(&self) {
        if let Self::Sync(s) = self {
            s.mark_taken();
        }
    }

    #[inline(always)]
    pub fn wake(self) {
        match self {
            Self::Async(s) => s.waker.wake(),
            Self::Sync(s) => s.notify(),
        }
    }

    #[inline(always)]
    pub fn wake_by_ref(&self) {
        match self {
            Self::Async(s) => s.waker.wake_by_ref(),
            Self::Sync(s) => s.notify(),
        }
    }

    /// Replace the stored waker (async signals only). Returns `true` when the
    /// waker was replaced. Sync signals have no waker; returns `false`.
    #[inline(always)]
    pub fn update_waker(&mut self, waker: &Waker) -> bool {
        match self {
            Self::Async(s) => s.update_waker(waker),
            Self::Sync(_) => false,
        }
    }
}

impl Clone for Signal {
    #[inline(always)]
    fn clone(&self) -> Self {
        match self {
            Self::Async(s) => Self::Async(s.clone()),
            Self::Sync(s) => Self::Sync(s.clone()),
        }
    }
}
