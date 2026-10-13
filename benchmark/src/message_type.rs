use std::sync::Arc;

#[derive(Clone)]
pub struct Message(Vec<u8>);

impl Message {
    /// Distinct allocation with every byte filled by the deterministic
    /// counter pattern (byte `i` holds `(i * 31) mod 256`): models real
    /// payloads, whose bytes are all touched. Same content as the `Touched`
    /// arm of [`AnyPayload::fresh`].
    pub fn new(size: usize) -> Self {
        let mut v = vec![0u8; size];
        fill_touched(&mut v);
        Self(v)
    }

    /// Send timestamp in nanos since the cell's `t0` (see [`AnyPayload::fresh`]),
    /// or `None` when this message is untracked (zero prefix) or too short
    /// to carry a stamp.
    pub fn send_stamp_nanos(&self) -> Option<u64> {
        stamp_of(&self.0)
    }
}

/// Read a send stamp from raw bytes: the first 8 hold nanos+1 in little
/// endian, with 0 reserved as the untracked sentinel.
fn stamp_of(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 8 {
        return None;
    }
    let raw = u64::from_le_bytes(bytes[0..8].try_into().expect("len >= 8"));
    // `then` (lazy): `raw - 1` must not evaluate for the zero sentinel.
    (raw != 0).then(|| raw - 1)
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

/// Which payload construction senders use. `Touched` allocates a distinct
/// `Vec` per send holding a deterministic counter pattern (template built
/// once per sender task, `memcpy` per send): models real payloads, whose
/// bytes are all touched. `Shared` clones one preallocated `Arc<Vec<u8>>`
/// per send: a channel-isolation control that removes allocator traffic,
/// not a model of real traffic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PayloadKind {
    Shared,
    Touched,
}

impl PayloadKind {
    pub fn token(self) -> &'static str {
        match self {
            PayloadKind::Shared => "shared",
            PayloadKind::Touched => "touched",
        }
    }

    pub fn from_env(s: &str) -> Result<Self, String> {
        match s.trim() {
            "shared" => Ok(PayloadKind::Shared),
            "touched" => Ok(PayloadKind::Touched),
            other => Err(format!(
                "unknown payload '{other}' (valid: shared, touched)"
            )),
        }
    }
}

/// Sampling config shared by one benchmark cell's sender and receiver tasks:
/// the `t0` baseline send-stamps count from, and the systematic sampling
/// stride. The stride derives from the cell's message count so every run
/// records the same tail budget (`DIST_RECORDED_TARGET` samples total, ~10
/// at/above p99.9) at any scale; tiny cells fall back to a census.
#[derive(Clone, Copy, Debug)]
pub struct DistCtx {
    pub t0: std::time::Instant,
    pub sample_every: usize,
}

/// Total recorded latency samples per cell at any message count.
pub const DIST_RECORDED_TARGET: usize = 10_000;

impl DistCtx {
    pub fn new(msg_no: usize) -> Self {
        Self {
            t0: std::time::Instant::now(),
            sample_every: (msg_no / DIST_RECORDED_TARGET).max(1),
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
    ///
    /// `seq` is this sender's local message counter and `dctx` the cell's
    /// sampling config: every `sample_every`-th message carries its send
    /// timestamp (nanos since `t0`, +1 so 0 stays the untracked sentinel) in
    /// the first 8 bytes, which are zeros otherwise. The counter is
    /// sender-local — no shared atomic on the send path.
    pub fn fresh(
        kind: PayloadKind,
        size: usize,
        cache: &mut Option<Arc<Vec<u8>>>,
        seq: u64,
        dctx: DistCtx,
    ) -> Self {
        match kind {
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
                        // Zero sentinel prefix: the receiver treats a nonzero
                        // stamp as tracked. Copy traffic is identical either
                        // way; only stamped messages pay one extra 8B store.
                        if v.len() >= 8 {
                            v[0..8].copy_from_slice(&[0; 8]);
                        }
                        let a = Arc::new(v);
                        *cache = Some(a.clone());
                        a
                    }
                };
                let mut v = template.as_ref().clone();
                if v.len() >= 8 && seq % dctx.sample_every as u64 == 0 {
                    let stamp = dctx.t0.elapsed().as_nanos() as u64 + 1;
                    v[0..8].copy_from_slice(&stamp.to_le_bytes());
                }
                AnyPayload::Owned(Message(v))
            }
        }
    }

    /// Send timestamp in nanos since the cell's `t0` (see [`AnyPayload::fresh`]),
    /// or `None` when this payload is untracked or too short to carry a stamp.
    pub fn send_stamp_nanos(&self) -> Option<u64> {
        match self {
            AnyPayload::Owned(m) => m.send_stamp_nanos(),
            AnyPayload::Shared(a) => stamp_of(a),
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
        assert_eq!(PayloadKind::Shared.token(), "shared");
        assert_eq!(PayloadKind::Touched.token(), "touched");
        assert_eq!(PayloadKind::from_env("shared").unwrap(), PayloadKind::Shared);
        assert_eq!(PayloadKind::from_env("touched").unwrap(), PayloadKind::Touched);
        assert!(PayloadKind::from_env("banana").is_err());
        assert!(PayloadKind::from_env("alloc").is_err(), "alloc mode removed");
    }

    #[test]
    fn dist_ctx_stride_adapts_to_message_count() {
        // Tail budget: ~10 samples at/above p99.9 at any scale; census below.
        assert_eq!(DistCtx::new(100_000).sample_every, 10);
        assert_eq!(DistCtx::new(10_000_000).sample_every, 1000);
        assert_eq!(DistCtx::new(100).sample_every, 1);
    }

    #[test]
    fn fresh_touched_content_matches_pattern() {
        let m = Message::new(300);
        assert_eq!(m.0.len(), 300);
        assert_eq!((m.0[0], m.0[1], m.0[9]), (0, 31, (9 * 31 % 256) as u8));
        // Unstamped (seq not a multiple of the stride): zero prefix plus the
        // shared pattern past it.
        let dctx = DistCtx::new(100_000);
        let mut cache = None;
        match AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 1, dctx) {
            AnyPayload::Owned(x) => {
                assert_eq!(x.0[0..8], [0; 8], "unstamped prefix stays zero");
                assert_eq!(x.0[8..], m.0[8..], "same pattern past the prefix");
                assert_eq!(x.send_stamp_nanos(), None);
            }
            _ => panic!("expected owned payload"),
        }
    }

    #[test]
    fn fresh_touched_stamps_every_stride_th_message() {
        let dctx = DistCtx::new(100_000);
        assert_eq!(dctx.sample_every, 10);
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 0, dctx);
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 1, dctx);
        // seq 0 is stamped: nonzero LE nanos, pattern intact past the prefix.
        let stamp = a.send_stamp_nanos().expect("seq 0 is sampled");
        assert!(stamp > 0);
        match (&a, &b) {
            (AnyPayload::Owned(x), AnyPayload::Owned(y)) => {
                assert_eq!(u64::from_le_bytes(x.0[0..8].try_into().unwrap()) - 1, stamp);
                assert_eq!(x.0[8..], y.0[8..], "pattern past the prefix");
                assert_eq!(y.0[0..8], [0; 8]);
                assert_ne!(x.0.as_ptr(), y.0.as_ptr(), "distinct allocations");
            }
            _ => panic!("expected owned payloads"),
        }
        assert_eq!(b.send_stamp_nanos(), None);
    }

    #[test]
    fn touched_payload_has_distinct_touched_content() {
        let dctx = DistCtx::new(100_000);
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 1, dctx);
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 2, dctx);
        assert_eq!((a.len(), b.len()), (300, 300));
        // Deterministic counter pattern past the stamp prefix: byte i holds
        // (i * 31) mod 256. Touched builds the template once per cache, then
        // copies it per send: distinct allocations, identical bytes.
        assert!(cache.is_some(), "touched mode caches the template");
        match (&a, &b) {
            (AnyPayload::Owned(x), AnyPayload::Owned(y)) => {
                assert_eq!(x.0[0..8], [0; 8]);
                assert_eq!(x.0[9], (9 * 31 % 256) as u8);
                assert_eq!(x.0[299], (299 * 31 % 256) as u8);
                assert_ne!(x.0.as_ptr(), y.0.as_ptr(), "distinct allocations");
            }
            _ => panic!("expected owned payloads"),
        }
    }

    #[test]
    fn touched_payload_reuses_template_across_sizes() {
        let dctx = DistCtx::new(100_000);
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 1, dctx);
        let template_ptr = cache.as_ref().unwrap().as_ptr();
        // Same size reuses the template; content stays identical.
        let b = AnyPayload::fresh(PayloadKind::Touched, 300, &mut cache, 2, dctx);
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
        let c = AnyPayload::fresh(PayloadKind::Touched, 16, &mut cache, 3, dctx);
        assert_eq!(c.len(), 16);
        assert_eq!(cache.as_ref().unwrap().len(), 16);
        match c {
            AnyPayload::Owned(x) => assert_eq!(x.0[9], (9 * 31 % 256) as u8),
            _ => panic!("expected owned payload"),
        }
    }

    #[test]
    fn shared_payload_reuses_one_allocation() {
        let dctx = DistCtx::new(100_000);
        let mut cache = None;
        let a = AnyPayload::fresh(PayloadKind::Shared, 16384, &mut cache, 0, dctx);
        let b = AnyPayload::fresh(PayloadKind::Shared, 16384, &mut cache, 1, dctx);
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
