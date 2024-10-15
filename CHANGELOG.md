# Changelog

## Version 0.4.0
- Implemented Stream and Sink traits for Receiver and Sender
- Added SendSink struct to wrap Sender for Sink implementation
- Implemented Sink trait from futures crate for SendSink
- Added RecvStream struct to wrap Receiver for Stream implementation
- Implemented Stream trait from futures crate for RecvStream
- Added helper methods to create SendSink and RecvStream instances

## Version 0.3.2
- Fixed bug in send methods to properly wake pending receivers
- Added convenience methods to SendFuture and RecvFuture:
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
