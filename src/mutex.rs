pub use StdMutex as Mutex;

#[derive(Debug)]
pub struct StdMutex<T>(std::sync::Mutex<T>);

pub type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

impl<T> StdMutex<T> {
    #[inline(always)]
    pub fn new(t: T) -> Self {
        Self(std::sync::Mutex::new(t))
    }

    #[inline(always)]
    pub fn lock(&self) -> MutexGuard<T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}
