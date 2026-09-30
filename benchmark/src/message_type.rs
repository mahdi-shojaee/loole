use std::fmt::{Debug, Display};
use std::sync::Arc;

pub struct Type<T: Length>(usize, T);

impl<T: Default + Length> From<usize> for Type<T> {
    #[inline(always)]
    fn from(value: usize) -> Self {
        Self(value, Default::default())
    }
}

impl<T: Length> Debug for Type<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("index: {}, len: {}", self.0, self.1.len()))
    }
}

impl<T: Length> Display for Type<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("index: {}, len: {}", self.0, self.1.len()))
    }
}

pub struct FilledArray<const SIZE: usize>([u8; SIZE]);

impl<const SIZE: usize> Default for FilledArray<SIZE> {
    #[inline(always)]
    fn default() -> Self {
        Self([0; SIZE])
    }
}

pub struct FilledVec<const SIZE: usize>(Vec<u8>);

impl<const SIZE: usize> Default for FilledVec<SIZE> {
    #[inline(always)]
    fn default() -> Self {
        Self(vec![0; SIZE])
    }
}

pub trait Length {
    fn len(&self) -> usize;
}

impl<const SIZE: usize> Length for FilledArray<SIZE> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<const SIZE: usize> Length for FilledVec<SIZE> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub type StackType<const SIZE: usize> = Type<FilledArray<SIZE>>;
pub type HeapType<const SIZE: usize> = Type<FilledVec<SIZE>>;

#[derive(Clone)]
pub struct Message(Vec<u8>);

impl Message {
    pub fn new(size: usize) -> Self {
        Self(vec![0; size])
    }
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("len: {}", self.0.len()))
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("len: {}", self.0.len()))
    }
}

/// Which payload construction senders use. `Alloc` is the historical
/// behavior (a fresh zeroed `Vec` per message: measures allocator +
/// channel). `Shared` clones one preallocated `Arc<Vec<u8>>` per send: a
/// channel-isolation control that removes allocator traffic, not a model of
/// real traffic. `Touched` allocates a distinct `Vec` per send and fills it
/// with a deterministic counter pattern: models real payloads, whose bytes
/// are all touched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PayloadKind {
    Alloc,
    Shared,
    Touched,
}

impl PayloadKind {
    pub fn token(self) -> &'static str {
        match self {
            PayloadKind::Alloc => "alloc",
            PayloadKind::Shared => "shared",
            PayloadKind::Touched => "touched",
        }
    }

    pub fn from_env(s: &str) -> Result<Self, String> {
        match s.trim() {
            "alloc" => Ok(PayloadKind::Alloc),
            "shared" => Ok(PayloadKind::Shared),
            "touched" => Ok(PayloadKind::Touched),
            other => Err(format!(
                "unknown payload '{other}' (valid: alloc, shared, touched)"
            )),
        }
    }
}

/// Channel payload for throughput cells. `Message` stays the owned variant so
/// all existing channel-type code keeps working by mechanical substitution.
#[derive(Clone, Debug)]
pub enum AnyPayload {
    Owned(Message),
    Shared(Arc<Vec<u8>>),
}

impl AnyPayload {
    /// Build one send unit. In `Shared` mode the allocation happens once per
    /// sender task (first call fills `cache`); every later call clones the
    /// `Arc` (refcount bump, no 16KB traffic). In `Touched` mode every call
    /// allocates distinctly and writes a deterministic counter pattern over
    /// all bytes (`cache` untouched).
    pub fn fresh(
        kind: PayloadKind,
        size: usize,
        cache: &mut Option<Arc<Vec<u8>>>,
    ) -> Self {
        match kind {
            PayloadKind::Alloc => AnyPayload::Owned(Message::new(size)),
            PayloadKind::Shared => {
                let arc = match cache {
                    Some(a) => a.clone(),
                    None => {
                        let a = Arc::new(vec![0u8; size]);
                        *cache = Some(a.clone());
                        a
                    }
                };
                AnyPayload::Shared(arc)
            }
            PayloadKind::Touched => {
                let mut v = vec![0u8; size];
                for (i, b) in v.iter_mut().enumerate() {
                    *b = (i.wrapping_mul(31) & 0xFF) as u8;
                }
                AnyPayload::Owned(Message(v))
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            AnyPayload::Owned(m) => m.0.len(),
            AnyPayload::Shared(v) => v.len(),
        }
    }
}

impl std::fmt::Display for AnyPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("len: {}", self.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_kind_tokens_and_env() {
        assert_eq!(PayloadKind::Alloc.token(), "alloc");
        assert_eq!(PayloadKind::Shared.token(), "shared");
        assert_eq!(PayloadKind::Touched.token(), "touched");
        assert_eq!(PayloadKind::from_env("alloc").unwrap(), PayloadKind::Alloc);
        assert_eq!(PayloadKind::from_env("shared").unwrap(), PayloadKind::Shared);
        assert_eq!(PayloadKind::from_env("touched").unwrap(), PayloadKind::Touched);
        assert!(PayloadKind::from_env("banana").is_err());
    }

    #[test]
    fn touched_payload_has_distinct_touched_content() {
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        assert_eq!((a.len(), b.len()), (300, 300));
        // Deterministic counter pattern: byte i holds (i * 31) mod 256.
        // Touched ignores the cache (every message is a distinct allocation).
        assert!(cache.is_none());
        match (&a, &b) {
            (AnyPayload::Owned(x), AnyPayload::Owned(y)) => {
                assert_eq!(x.0[0], 0);
                assert_eq!(x.0[1], 31);
                assert_eq!(x.0[9], (9 * 31 % 256) as u8);
                assert_eq!(x.0[299], (299 * 31 % 256) as u8);
                assert_ne!(x.0.as_ptr(), y.0.as_ptr(), "distinct allocations");
            }
            _ => panic!("expected owned payloads"),
        }
    }

    #[test]
    fn alloc_payload_builds_fresh_zeroed_vecs() {
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Alloc, 16, &mut cache);
        let b = AnyPayload::fresh(PayloadKind::Alloc, 16, &mut cache);
        assert_eq!((a.len(), b.len()), (16, 16));
        assert!(cache.is_none(), "alloc mode never touches the cache");
    }

    #[test]
    fn shared_payload_reuses_one_allocation() {
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Shared, 16384, &mut cache);
        let b = AnyPayload::fresh(PayloadKind::Shared, 16384, &mut cache);
        assert_eq!((a.len(), b.len()), (16384, 16384));
        match (&a, &b) {
            (AnyPayload::Shared(x), AnyPayload::Shared(y)) => {
                assert!(Arc::ptr_eq(x, y), "same backing allocation");
            }
            _ => panic!("expected shared payloads"),
        }
        assert!(cache.is_some());
    }
}
