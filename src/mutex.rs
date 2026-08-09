pub use StdMutex as Mutex;

#[derive(Debug)]
pub struct StdMutex<T>(std::sync::Mutex<T>);

pub type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

/// How many pure `spin_loop()` pauses to perform after a failed `try_lock`
/// before a single retry and, failing that, the blocking lock. The pause phase
/// performs NO atomic ops: a held critical section in the channel hot path is
/// ~20-50 ns, so ~200-300 ns of `pause` (hint::spin_loop on x86) lets a
/// contender on a tight SPSC handoff acquire without the ~1 us futex syscall.
/// Crucially, the spin does not touch the mutex's cache line, so it does not
/// cause the CAS storm that a `try_lock` retry loop triggers under 1000-thread
/// MPMC contention; under heavy contention every thread still does at most two
/// `try_lock` attempts and then parks on the std futex, which is well-behaved
/// for the MPMC benchmark rows.
const SPIN_PAUSES: u32 = 16;

impl<T> StdMutex<T> {
    #[inline(always)]
    pub fn new(t: T) -> Self {
        Self(std::sync::Mutex::new(t))
    }

    #[inline(always)]
    pub fn lock(&self) -> MutexGuard<'_, T> {
        if let Ok(guard) = self.0.try_lock() {
            return guard;
        }
        // Contended: pause purely (no atomics), then retry once. If the holder
        // released during the pause (common in an SPSC handoff) we acquire
        // without the futex syscall; otherwise fall through to the blocking
        // std lock, which parks on the futex just like an uncontended std
        // mutex would.
        for _ in 0..SPIN_PAUSES {
            core::hint::spin_loop();
        }
        if let Ok(guard) = self.0.try_lock() {
            return guard;
        }
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}
