pub use {std::sync::MutexGuard, StdMutex as Mutex};

#[derive(Debug)]
pub struct StdMutex<T>(std::sync::Mutex<T>);

impl<T> StdMutex<T> {
    #[inline(always)]
    pub fn new(t: T) -> Self {
        Self(std::sync::Mutex::new(t))
    }

    #[inline(always)]
    pub fn lock(&self) -> std::sync::MutexGuard<T> {
        self.0.lock().map_or_else(|e| e.into_inner(), |v| v)
    }
}
