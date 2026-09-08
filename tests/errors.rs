//! Tests for the error-type API surface: `into_inner`, `From` conversions,
//! `Display`, and `Debug` implementations of all error types.

use std::time::Duration;

use loole::{
    bounded, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError, TrySendError,
};

/// A payload type that deliberately does not implement `Debug`, to prove the
/// custom `Debug` impls of the send-side errors.
struct NoDebug;

#[test]
fn send_error_into_inner() {
    let (tx, rx) = bounded::<i32>(1);
    drop(rx);
    let err = tx.send(42).unwrap_err();
    assert_eq!(err.into_inner(), 42);
}

#[test]
fn try_send_error_into_inner_full() {
    let (tx, _rx) = bounded(1);
    tx.send(1).unwrap();
    let err = tx.try_send(2).unwrap_err();
    assert_eq!(err, TrySendError::Full(2));
    assert_eq!(err.into_inner(), 2);
}

#[test]
fn try_send_error_into_inner_disconnected() {
    let (tx, rx) = bounded::<&str>(1);
    drop(rx);
    let err = tx.try_send("payload").unwrap_err();
    assert_eq!(err, TrySendError::Disconnected("payload"));
    assert_eq!(err.into_inner(), "payload");
}

#[test]
fn send_timeout_error_into_inner_timeout() {
    let (tx, _rx) = bounded(1);
    tx.send(1).unwrap();
    let err = tx.send_timeout(2, Duration::from_millis(10)).unwrap_err();
    assert_eq!(err, SendTimeoutError::Timeout(2));
    assert_eq!(err.into_inner(), 2);
}

#[test]
fn send_timeout_error_into_inner_disconnected() {
    let (tx, rx) = bounded::<Vec<u8>>(1);
    drop(rx);
    let err = tx
        .send_timeout(vec![1, 2, 3], Duration::from_millis(10))
        .unwrap_err();
    assert_eq!(err, SendTimeoutError::Disconnected(vec![1, 2, 3]));
    assert_eq!(err.into_inner(), vec![1, 2, 3]);
}

#[test]
fn from_send_error_for_try_send_error() {
    let err: SendError<u32> = SendError(7);
    let converted: TrySendError<u32> = err.into();
    assert_eq!(converted, TrySendError::Disconnected(7));
}

#[test]
fn from_send_error_for_send_timeout_error() {
    let err: SendError<u32> = SendError(9);
    let converted: SendTimeoutError<u32> = err.into();
    assert_eq!(converted, SendTimeoutError::Disconnected(9));
}

#[test]
fn from_recv_error_for_try_recv_error() {
    let err: RecvError = RecvError::Disconnected;
    let converted: TryRecvError = err.into();
    assert_eq!(converted, TryRecvError::Disconnected);
}

#[test]
fn error_display_messages() {
    assert_eq!(
        RecvError::Disconnected.to_string(),
        "receiving on a closed channel"
    );
    assert_eq!(SendError::<()>(()).to_string(), "sending on a closed channel");
    assert_eq!(
        TrySendError::<()>::Full(()).to_string(),
        "sending on a full channel"
    );
    assert_eq!(
        TrySendError::<()>::Disconnected(()).to_string(),
        "sending on a closed channel"
    );
    assert_eq!(
        TryRecvError::Empty.to_string(),
        "receiving on an empty channel"
    );
    assert_eq!(
        TryRecvError::Disconnected.to_string(),
        "channel is empty and closed"
    );
    assert_eq!(
        RecvTimeoutError::Timeout.to_string(),
        "timed out waiting on a channel"
    );
    assert_eq!(
        RecvTimeoutError::Disconnected.to_string(),
        "channel is empty and closed"
    );
    assert_eq!(
        SendTimeoutError::<()>::Timeout(()).to_string(),
        "timed out sending on a full channel"
    );
    assert_eq!(
        SendTimeoutError::<()>::Disconnected(()).to_string(),
        "sending on a closed channel"
    );
}

#[test]
fn error_debug_formatting() {
    // The send-side errors' `Debug` impls must not require the payload to be `Debug`.
    assert_eq!(format!("{:?}", SendError(NoDebug)), "SendError(..)");
    assert_eq!(format!("{:?}", TrySendError::Full(NoDebug)), "Full(..)");
    assert_eq!(
        format!("{:?}", TrySendError::Disconnected(NoDebug)),
        "Disconnected(..)"
    );
    assert_eq!(
        format!("{:?}", SendTimeoutError::Timeout(NoDebug)),
        "SendTimeoutError(..)"
    );
    assert_eq!(format!("{:?}", RecvError::Disconnected), "Disconnected");
    assert_eq!(format!("{:?}", TryRecvError::Empty), "Empty");
    assert_eq!(format!("{:?}", RecvTimeoutError::Timeout), "Timeout");

    // Errors are usable as `dyn Error` regardless of the payload type.
    fn assert_error<E: std::error::Error>(_: &E) {}
    assert_error(&SendError(NoDebug));
    assert_error(&TrySendError::Full(NoDebug));
    assert_error(&SendTimeoutError::Timeout(NoDebug));
    assert_error(&RecvError::Disconnected);
    assert_error(&TryRecvError::Empty);
    assert_error(&RecvTimeoutError::Timeout);
}

#[test]
fn error_equality_and_clonability() {
    fn assert_copy_clone_eq<T: Copy + Clone + PartialEq + std::fmt::Debug>(a: T, b: T) {
        assert_eq!(a, b);
    }
    assert_copy_clone_eq(RecvError::Disconnected, RecvError::Disconnected);
    assert_copy_clone_eq(TryRecvError::Empty, TryRecvError::Empty);
    assert_copy_clone_eq(TryRecvError::Disconnected, TryRecvError::Disconnected);
    assert_copy_clone_eq(RecvTimeoutError::Timeout, RecvTimeoutError::Timeout);
    assert_copy_clone_eq(
        RecvTimeoutError::Disconnected,
        RecvTimeoutError::Disconnected,
    );

    // Send-side errors are `Clone + PartialEq`.
    fn assert_clone_eq<T: Clone + PartialEq + std::fmt::Debug>(a: &T, b: &T) {
        assert_eq!(a, b);
        assert_eq!(a.clone(), *b);
    }
    assert_clone_eq(&SendError(1), &SendError(1));
    assert_clone_eq(&TrySendError::Full(1), &TrySendError::Full(1));
    assert_clone_eq(&TrySendError::Disconnected(1), &TrySendError::Disconnected(1));
    assert_clone_eq(&SendTimeoutError::Timeout(1), &SendTimeoutError::Timeout(1));
    assert_clone_eq(
        &SendTimeoutError::Disconnected(1),
        &SendTimeoutError::Disconnected(1),
    );

    // Variants differ from each other.
    assert_ne!(TryRecvError::Empty, TryRecvError::Disconnected);
    assert_ne!(TrySendError::<u8>::Full(1), TrySendError::Disconnected(1));
    assert_ne!(
        SendTimeoutError::<u8>::Timeout(1),
        SendTimeoutError::Disconnected(1)
    );
}
