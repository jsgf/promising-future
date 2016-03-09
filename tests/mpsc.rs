// Check out the properties of mpsc queues
use std::sync::mpsc::{channel, sync_channel, SendError, TryRecvError};
use std::thread;
use std::mem;

#[test]
fn basic() {
    let (tx, rx) = channel();

    tx.send(123).expect("send");
    assert_eq!(rx.recv().ok().unwrap(), 123);
}

#[test]
fn txdrop() {
    let (_, rx) = channel::<()>();

    assert!(rx.recv().is_err());
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn rxdrop() {
    let (tx, _) = channel();

    // double send with dead rx works the second time...
    //assert_eq!(tx.send(123), Err(SendError(123)));
    assert_eq!(tx.send(123), Err(SendError(123)));
}

#[test]
fn buffer() {
    let (tx, rx) = channel();

    tx.send(123).expect("tx send");
    mem::drop(tx);

    assert_eq!(rx.recv(), Ok(123))
}

#[test]
fn sync_basic() {
    let (tx, rx) = sync_channel(1);

    tx.send(123).expect("send");
    assert_eq!(rx.recv().ok().unwrap(), 123);
}

#[test]
fn sync_txdrop() {
    let (_, rx) = sync_channel::<()>(1);

    assert!(rx.recv().is_err());
}

#[test]
fn sync_rxdrop() {
    let (tx, _) = sync_channel(1);

    assert!(tx.send(123).is_err());
}

#[test]
fn sync_buffer() {
    let (tx, rx) = sync_channel(1);

    tx.send(123).expect("tx send");
    mem::drop(tx);

    assert_eq!(rx.recv(), Ok(123))
}

#[test]
fn sync0_basic() {
    let (tx, rx) = sync_channel(0);

    thread::spawn(move || tx.send(123));
    assert_eq!(rx.recv().ok().unwrap(), 123);
}

#[test]
fn sync0_txdrop() {
    let (_, rx) = sync_channel::<()>(0);

    assert!(rx.recv().is_err());
}

#[test]
fn sync0_rxdrop() {
    let (tx, _) = sync_channel(0);

    assert!(tx.send(123).is_err());
}
