//! Precise contract tests for the missing channel-state APIs, mirroring flume
//! 0.12.0 EXACTLY (the version loole tracks). Ranked #1 + #2 missing APIs in
//! the top-100 reverse-dependency scan (`sender_count`/`receiver_count` block
//! ~27.5M downloads: lapin, zenoh ×6, velo ×5, web-rwkv; `is_disconnected`
//! blocks ~12.8M across 20 crates: mdns-sd, goose, compio ×3, songbird,
//! sea-streamer ×3, dora ×3, …). Full ranking:
//! `/home/mahdi/projects/loole/optimize/loole-missing-apis.md`.
//!
//! Flume reference semantics (flume 0.12.0 `src/lib.rs`), read from source:
//! - ONE shared `disconnected: AtomicBool` latch per channel, set by
//!   `disconnect_all()` when the LAST sender drops OR the LAST receiver drops
//!   (`Drop for Sender` / `Drop for Receiver` fire it exactly on the
//!   `fetch_sub == 1` transition). BOTH `Sender::is_disconnected()` and
//!   `Receiver::is_disconnected()` read this same flag, so:
//!   (a) `Receiver::is_disconnected()` is true IMMEDIATELY when the last
//!       sender drops, even with messages still queued — queued messages stay
//!       receivable ("will not prevent receivers receiving msgs that have
//!       already been sent");
//!   (b) `Sender::is_disconnected()` is true IMMEDIATELY when the last
//!       receiver drops, even with messages still queued;
//!   (c) partial drops never set it (2→1 on either side keeps `false`).
//! - `sender_count` / `receiver_count` are plain `AtomicUsize`s starting at 1,
//!   `fetch_add` on `clone`, `fetch_sub` on `drop` (all `Relaxed`); the count
//!   INCLUDES the handle it is called on; `WeakSender` (not covered here —
//!   zero hits in the scan) does not affect counts, and `upgrade()` refuses
//!   to resurrect from 0. No cross-thread exactness is asserted here: flume
//!   documents no ordering guarantee for concurrent clone/drop vs reads.
//! - flume has NO `close()`: its interaction with these methods is loole-side
//!   design freedom, EXCEPT the invariant pinned below that live handles keep
//!   their counts after `close()`.
//!
//! These tests currently FAIL TO COMPILE (methods do not exist) — that is the
//! correct RED state for missing APIs. DO NOT "fix" them by weakening
//! assertions — they encode flume's contract verbatim.

use loole::{bounded, unbounded};

// ---------------------------------------------------------------------------
// sender_count
// ---------------------------------------------------------------------------

/// Fresh channel: exactly one sender, visible from both ends.
#[test]
fn sender_count_fresh_channel_is_one() {
    let (tx, rx) = unbounded::<()>();
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.sender_count(), 1);

    let (tx, rx) = bounded::<()>(8);
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.sender_count(), 1);

    let (tx, rx) = bounded::<()>(0);
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.sender_count(), 1);
}

/// Clones raise the count; drops lower it; both ends always agree.
#[test]
fn sender_count_tracks_clones_and_drops() {
    let (tx, rx) = unbounded::<()>();
    let tx2 = tx.clone();
    let tx3 = tx.clone();
    assert_eq!(tx.sender_count(), 3);
    assert_eq!(rx.sender_count(), 3);
    drop(tx2);
    assert_eq!(tx.sender_count(), 2);
    assert_eq!(rx.sender_count(), 2);
    drop(tx3);
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.sender_count(), 1);
}

/// Last sender dropped → receivers observe 0.
#[test]
fn sender_count_zero_after_last_sender_dropped() {
    let (tx, rx) = unbounded::<()>();
    let tx2 = tx.clone();
    drop(tx);
    drop(tx2);
    assert_eq!(rx.sender_count(), 0);
}

/// Counts are orthogonal to capacity and to queued messages.
#[test]
fn sender_count_ignores_capacity_and_queue() {
    let (tx, rx) = bounded(2);
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.sender_count(), 1);
    let tx2 = tx.clone();
    assert_eq!(rx.sender_count(), 2);
    drop(tx2);
    assert_eq!(rx.sender_count(), 1);
}

// ---------------------------------------------------------------------------
// receiver_count
// ---------------------------------------------------------------------------

/// Fresh channel: exactly one receiver, visible from both ends.
#[test]
fn receiver_count_fresh_channel_is_one() {
    let (tx, rx) = unbounded::<()>();
    assert_eq!(tx.receiver_count(), 1);
    assert_eq!(rx.receiver_count(), 1);

    let (tx, rx) = bounded::<()>(8);
    assert_eq!(tx.receiver_count(), 1);
    assert_eq!(rx.receiver_count(), 1);
}

/// Clones raise the count; drops lower it; both ends always agree.
#[test]
fn receiver_count_tracks_clones_and_drops() {
    let (tx, rx) = unbounded::<()>();
    let rx2 = rx.clone();
    assert_eq!(tx.receiver_count(), 2);
    assert_eq!(rx.receiver_count(), 2);
    drop(rx2);
    assert_eq!(tx.receiver_count(), 1);
    assert_eq!(rx.receiver_count(), 1);
}

/// Last receiver dropped → senders observe 0.
#[test]
fn receiver_count_zero_after_last_receiver_dropped() {
    let (tx, rx) = unbounded::<()>();
    drop(rx);
    assert_eq!(tx.receiver_count(), 0);
}

// ---------------------------------------------------------------------------
// is_disconnected — Sender side (true iff all receivers dropped)
// ---------------------------------------------------------------------------

/// Fresh channels are connected, at every capacity including rendezvous.
#[test]
fn sender_not_disconnected_while_receiver_alive() {
    let (tx, _rx) = unbounded::<()>();
    assert!(!tx.is_disconnected());

    let (tx, _rx) = bounded::<()>(8);
    assert!(!tx.is_disconnected());

    let (tx, _rx) = bounded::<()>(0);
    assert!(!tx.is_disconnected());
}

/// Dropping the sole receiver disconnects immediately — even with messages
/// still queued (flume latches on last-drop, not on drain).
#[test]
fn sender_disconnected_when_last_receiver_dropped_with_queued_messages() {
    let (tx, rx) = bounded(4);
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    assert!(!tx.is_disconnected());
    drop(rx);
    assert!(tx.is_disconnected());
}

/// Partial receiver drops keep the sender connected; only the last one latches.
#[test]
fn sender_connected_until_last_receiver_dropped() {
    let (tx, rx) = unbounded::<()>();
    let rx2 = rx.clone();
    drop(rx2);
    assert!(!tx.is_disconnected());
    drop(rx);
    assert!(tx.is_disconnected());
}

// ---------------------------------------------------------------------------
// is_disconnected — Receiver side (true iff all senders dropped)
// ---------------------------------------------------------------------------

/// Fresh channels are connected, at every capacity.
#[test]
fn receiver_not_disconnected_while_sender_alive() {
    let (_tx, rx) = unbounded::<()>();
    assert!(!rx.is_disconnected());

    let (_tx, rx) = bounded::<()>(8);
    assert!(!rx.is_disconnected());
}

/// Dropping the sole sender disconnects immediately — even with messages
/// still queued — while those messages stay receivable (flume `disconnect_all`
/// doc: "this will not prevent receivers receiving msgs that have already
/// been sent").
#[test]
fn receiver_disconnected_when_last_sender_dropped_with_queued_messages() {
    let (tx, rx) = unbounded();
    tx.send(10).unwrap();
    tx.send(20).unwrap();
    assert!(!rx.is_disconnected());
    drop(tx);
    assert!(rx.is_disconnected());
    // queue is NOT drained by disconnecting:
    assert_eq!(rx.try_recv(), Ok(10));
    assert_eq!(rx.try_recv(), Ok(20));
    assert!(rx.is_disconnected());
}

/// Partial sender drops keep the receiver connected; only the last one latches.
#[test]
fn receiver_connected_until_last_sender_dropped() {
    let (tx, rx) = unbounded::<()>();
    let tx2 = tx.clone();
    drop(tx2);
    assert!(!rx.is_disconnected());
    drop(tx);
    assert!(rx.is_disconnected());
}

/// Rendezvous channels follow the same latch rules (disconnect is capacity
/// independent).
#[test]
fn receiver_disconnect_on_rendezvous_channel() {
    let (tx, rx) = bounded::<()>(0);
    assert!(!rx.is_disconnected());
    assert!(!tx.is_disconnected());
    drop(tx);
    assert!(rx.is_disconnected());
}

// ---------------------------------------------------------------------------
// Invariants across the API
// ---------------------------------------------------------------------------

/// `close()` drops no handles, so counts are unaffected (flume has no
/// `close()`; this pins the only sane reading for loole's extra API).
#[test]
fn close_does_not_change_counts() {
    let (tx, rx) = bounded::<()>(4);
    let tx2 = tx.clone();
    let rx2 = rx.clone();
    tx.close();
    assert_eq!(tx.sender_count(), 2);
    assert_eq!(rx.sender_count(), 2);
    assert_eq!(tx.receiver_count(), 2);
    assert_eq!(rx.receiver_count(), 2);
    drop(tx2);
    drop(rx2);
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(rx.receiver_count(), 1);
}

/// Counts and disconnect agree at the terminal state: no senders + no
/// receivers is observable from whichever handle drops last is gone — so
/// check the fully-drained sequence from a surviving clone.
#[test]
fn counts_and_disconnect_agree_through_drop_sequence() {
    let (tx, rx) = unbounded::<()>();
    let tx2 = tx.clone();
    let rx2 = rx.clone();
    assert_eq!((tx.sender_count(), tx.receiver_count()), (2, 2));
    assert!(!tx.is_disconnected());
    assert!(!rx.is_disconnected());
    drop(tx2);
    drop(rx2);
    assert_eq!((tx.sender_count(), tx.receiver_count()), (1, 1));
    assert!(!tx.is_disconnected());
    assert!(!rx.is_disconnected());
    drop(rx);
    assert_eq!(tx.sender_count(), 1);
    assert_eq!(tx.receiver_count(), 0);
    assert!(tx.is_disconnected());
}
