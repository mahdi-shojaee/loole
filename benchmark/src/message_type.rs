use std::sync::Arc;

#[derive(Clone)]
pub struct Message(Vec<u8>);

impl Message {
    pub fn new(size: usize) -> Self {
        Self(vec![0; size])
    }

    /// Distinct allocation with every byte filled by the deterministic
    /// counter pattern (byte `i` holds `(i * 31) mod 256`): models real
    /// payloads, whose bytes are all touched. Same content as the `Touched`
    /// arm of [`AnyPayload::fresh`].
    pub fn new_touched(size: usize) -> Self {
        let mut v = vec![0u8; size];
        fill_touched(&mut v);
        Self(v)
    }
}

/// Fill every byte with the deterministic touched-content pattern.
fn fill_touched(v: &mut [u8]) {
    for (i, b) in v.iter_mut().enumerate() {
        *b = (i.wrapping_mul(31) & 0xFF) as u8;
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
/// real traffic. `Touched` allocates a distinct `Vec` per send holding a
/// deterministic counter pattern (template built once per sender task,
/// `memcpy` per send): models real payloads, whose bytes are all touched.
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
    /// `Arc` (refcount bump, no 16KB traffic). In `Touched` mode the patterned
    /// template is built once per sender task into `cache` and every call
    /// copies it into a fresh `Vec` (`malloc` + `memcpy`, no scalar fill
    /// loop): all messages stay distinct allocations with identical touched
    /// bytes, and the per-message generation cost — sender-side overhead
    /// outside the channel that dilutes the measured differences — is kept
    /// minimal.
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
                let template = match cache {
                    Some(a) if a.len() == size => a.clone(),
                    _ => {
                        let mut v = vec![0u8; size];
                        fill_touched(&mut v);
                        let a = Arc::new(v);
                        *cache = Some(a.clone());
                        a
                    }
                };
                AnyPayload::Owned(Message(template.as_ref().clone()))
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
    fn new_touched_matches_fresh_touched_content() {
        let m = Message::new_touched(300);
        assert_eq!(m.0.len(), 300);
        assert_eq!((m.0[0], m.0[1], m.0[9]), (0, 31, (9 * 31 % 256) as u8));
        let mut cache = None;
        match AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache) {
            AnyPayload::Owned(x) => assert_eq!(x.0, m.0, "same pattern"),
            _ => panic!("expected owned payload"),
        }
    }

    #[test]
    fn touched_payload_has_distinct_touched_content() {
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        assert_eq!((a.len(), b.len()), (300, 300));
        // Deterministic counter pattern: byte i holds (i * 31) mod 256.
        // Touched builds the template once per cache, then copies it per
        // send: every message is a distinct allocation with identical bytes.
        assert!(cache.is_some(), "touched mode caches the template");
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
    fn touched_payload_reuses_template_across_sizes() {
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        let template_ptr = cache.as_ref().unwrap().as_ptr();
        // Same size reuses the template; content stays identical.
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache);
        assert!(std::ptr::eq(
            cache.as_ref().unwrap().as_ptr(),
            template_ptr
        ));
        match (&a, &b) {
            (AnyPayload::Owned(x), AnyPayload::Owned(y)) => {
                assert_eq!(x.0, y.0, "identical bytes");
                assert_ne!(x.0.as_ptr(), y.0.as_ptr(), "distinct allocations");
            }
            _ => panic!("expected owned payloads"),
        }
        // Size change rebuilds the template instead of slicing wrongly.
        let c = AnyPayload::fresh(PayloadKind::Touched, 16, &mut cache);
        assert_eq!(c.len(), 16);
        assert_eq!(cache.as_ref().unwrap().len(), 16);
        match c {
            AnyPayload::Owned(x) => assert_eq!(x.0[9], (9 * 31 % 256) as u8),
            _ => panic!("expected owned payload"),
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
