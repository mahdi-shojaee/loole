# Changelog

## Version 0.5.1
- Fixed `SendSink` soundness under concurrent `forward()`: `poll_ready` now flushes the buffered send instead of reporting always-ready (fixes `forward()` delivering only the last item)
- Fixed `SendSink::poll_close` to release only its own sender handle instead of closing the shared channel (fixes spurious `SendError` under concurrent sinks)

## Version 0.5.0
- Performance: `send_async`/`recv_async` (and `sink`/`stream`) now return futures that borrow the sender/receiver instead of cloning it, removing four atomic reference-count operations per message (SPSC async-async bounded(100): up to +39% at 32 B, +7% at 16384 B on P-cores)
- Added `Sender::into_send_async` and `Receiver::into_recv_async`: owned `'static` futures that keep the channel open until dropped, for `tokio::spawn` and stored futures (the previous `send_async`/`recv_async` behavior)
- Behavior change: a borrowed `SendFuture`/`RecvFuture` no longer keeps the channel open — dropping all senders/receivers while one is pending closes the channel and the future resolves with an error
- `SendSink`/`RecvStream` and `Sender::sink`/`Receiver::stream` are now lifetime-parameterized (`SendSink<'a, T>`, `RecvStream<'a, T>`); `into_sink`/`into_stream` are unchanged in behavior
- Performance: lock-free consumed-park confirmation — a receiver that consumes a parked send publishes the entry's id on a per-channel watermark, so the woken `SendFuture` confirms with one atomic load instead of re-acquiring the channel mutex (which contended with the receiver's pop loop and could futex-stall the runtime worker, de-phasing the SPSC async-async pipeline on large buffered channels and halving b(100) 16KB throughput)
- Performance: single-waiter inline fast path in the pending queues — with exactly one parked waiter (the SPSC steady state) the entry lives in an inline slot with no hashing or table probe (previously 3+ `HashMap` ops per park/wake)
- Performance: a parked `SendFuture` now stores a single waker clone (in its queue entry, the future keeps just the id) with in-place refresh on re-poll, and bounded channels preallocate the message ring
- Tests: added `async_park_confirm` — watermark confirmation paths (consumed fast path, close fallback, FIFO monotonicity across multiple parked senders, cancellation, drain, sink supersede, stream receiver on a full channel, b100 SPSC async-async stress)
- Behavior change: sync `send`/`recv` on bounded channels with capacity 2-16 no longer spin before parking — the spin-then-park handoff now applies only to rendezvous-like channels (capacity 0/1) with a single peer. Interleaved A/B on the merged stack (i7-13700K, pinned CPUs 0-7, criterion 256B/alloc n=100000, 5 repeats): the wider cap<=16 spin gate regressed b10 SPSC sync-sync throughput -23.1% (4.05M vs 3.11M msg/s median, 0/5 repeats) and b10 sync-mixed rows -8.0%/-15.4%, outweighing its ~2.8x b10 ping-pong latency gain, so the rendezvous-only gate stands
- Performance: rewrote channel internals — wake one parked receiver per send instead of all, plain VecDeque message buffer, rendezvous hand-off for parked sync receivers, lazy pending-queue ids, `taken`-flag send confirmation, spin-then-park sync ops, identity-hashed pending queues, and close detection via `Arc` drop instead of counters
- Behavior change: dropping a pending `SendFuture` now cancels the send (the queued message is removed and destroyed) instead of delivering it anyway
- Fixed: polling a `SendFuture` after completion no longer panics
- Fixed: past deadlines given to `send_deadline`/`recv_deadline` behave like a zero timeout instead of panicking
- Fixed send/recv edge-case semantics (A1-A5) and added coverage-gap tests (cancellation, close, deadline, drain, errors, handles, select-send, sink)
- Fixed: a `RecvFuture` dropped after a sender claimed its registration but before being re-polled (e.g. a losing `select!` branch) now wakes the next parked receiver, so the delivered message is not stranded while receivers sleep
- Fixed: `send_timeout` with extreme durations (e.g. `Duration::MAX`) no longer panics on `Instant` overflow
- Fixed: `recv_timeout` no longer misses a message delivered exactly at the deadline in rendezvous-with-hook configurations
- Overhauled benchmarks: criterion-driven suite with auto-updated charts
- Restored Apache-2.0 licensing alongside MIT (project is dual-licensed MIT OR Apache-2.0 again, matching the declaration that Cargo.toml/crates.io kept all along)

## Version 0.4.1
- Performance: Added Sender/ReceiverInner types to consolidate two AtomicUsizes behind a single Arc (PR #6, thanks to [cod10129](https://github.com/cod10129))
- Fixed send methods to properly check completion status before returning success
- Refactored Queue implementation to use HashMap for storage
- Fixed Clippy mismatched_lifetime_syntaxes warnings by adding explicit lifetime annotations
- Updated benchmarks and dependencies

## Version 0.4.0
- Fixed bug in send methods to properly wake pending receivers
- Implemented Stream and Sink traits for Receiver and Sender
- Added SendSink struct to wrap Sender for Sink implementation
- Implemented Sink trait from futures crate for SendSink
- Added RecvStream struct to wrap Receiver for Stream implementation
- Implemented Stream trait from futures crate for RecvStream
- Added convenience methods to SendFuture, RecvFuture, SendSink and RecvStream:
  - is_closed(): Check if the channel is closed
  - is_empty(): Check if the channel is empty
  - is_full(): Check if the channel is full
  - len(): Get the number of messages in the channel
  - capacity(): Get the channel capacity

## Version 0.3.1
- Added unit tests for Queue implementation
- Enhanced error handling and reporting in benchmark files
- Updated license to MIT (removed Apache)
- Improved readability of benchmark code
- Fixed recv_timeout memory leak (Issue #4) (Thanks to [soloist-v](https://github.com/soloist-v) for the report)

## Version 0.3.0
- Added close and is_closed methods for Sender and Receiver
- Fixed issue with pending async senders erroring with mismatched values (PR #3) (Thanks to [asonix](https://github.com/asonix))

## Version 0.2.1
- Updated benchmark charts
- Inlined some Queue methods

## Version 0.2.0
- Added Receiver::drain method

## Version 0.1.16 and earlier
- Improved performance for bounded(0) channels
- Implemented Default for Queue
- Fixed: Prevent additional wake on zero-buffer channels
- Fixed: Wake recently entered pending send into buffer
- Updated criterion benchmarks
- Improved recv_async performance
- Implemented Debug for Sender and Receiver
- Added Criterion benchmarks
- Added benchmark charts to README
- Added command to update benchmark charts (Issue #1)
- Implemented Drop for RecvFuture
- Initial implementation of the channel
