use std::sync::Arc;
use std::sync::mpsc::{Sender, Receiver, channel};
use std::iter::FromIterator;

use super::Pollresult::*;
use super::Future;

use cvmx::CvMx;

/// Stream of multiple `Future`s
///
/// A `FutureStream` can be used to wait for multiple `Future`s, and return them incrementally as
/// they are resolved.
///
/// It implements an iterator over completed `Future`s, and can be constructed from an iterator of
/// `Future`s.
///
/// May be cloned and the clones passed to other threads so that `Future`s may be added from multiple
/// threads.
#[derive(Clone)]
pub struct FutureStream<T: Send> {
    tx: Sender<Option<T>>,                  // values from Promise
    inner: Arc<CvMx<FutureStreamInner<T>>>, // set of waited-for futures
}

/// Waiter for `Future`s in a `FutureStream`.
///
/// A singleton waiter for `Future`s, associated with a specific `FutureStream`. This may be used
/// in a multithreaded environment to wait for `Futures` to resolve while other threads fulfill
/// `Promises` and add new `Future`s to the `FutureStream`.
///
/// ```
/// # use ::promising_future::{Future,FutureStream};
/// # let future = Future::with_value(());
/// let fs = FutureStream::new();
/// fs.add(future);
/// // ...
/// let mut waiter = fs.waiter();
/// while let Some(future) = waiter.wait() {
///     match future.value() {
///       None => (),         // Future unfulfilled
///       Some(val) => val,
///     }
/// }
/// ```
///
/// It may also be converted into an `Iterator` over the values yielded by resolved `Future`s
/// (unfulfilled `Promise`s are ignored).
///
/// ```
/// # use ::promising_future::{Future,FutureStream};
/// # let fut1 = Future::with_value(());
/// let fs = FutureStream::new();
/// fs.add(fut1);
/// for val in fs.waiter() {
///    // ...
/// }
/// ```
pub struct FutureStreamWaiter<'a, T: Send + 'a> {
    fs: &'a FutureStream<T>,
    rx: Option<Receiver<Option<T>>>,        // Option so that Drop can remove it
}

struct FutureStreamInner<T: Send> {
    pending: usize,
    rx: Option<Receiver<Option<T>>>,        // value receiver (if not passed to a waiter)
}

impl<T: Send> FutureStream<T> {
    pub fn new() -> FutureStream<T> {
        let (tx, rx) = channel();
        let inner = FutureStreamInner {
            rx: Some(rx),
            pending: 0,
        };

        FutureStream {
            tx: tx,
            inner: Arc::new(CvMx::new(inner)),
        }
    }

    /// Add a `Future` to the stream.
    pub fn add(&self, fut: Future<T>) where T: 'static {
        let mut inner = self.inner.mx.lock().unwrap();
        let tx = self.tx.clone();

        inner.pending += 1;
        // If `tx.send()` fails, then it just means the waiter/FutureStream has gone away
        fut.callback_unit(move |v| { let _ = tx.send(v); })
    }

    /// Return number of outstanding `Future`s.
    pub fn outstanding(&self) -> usize {
        self.inner.mx.lock().unwrap().pending
    }

    /// Return a singleton `FutureStreamWaiter`. If one already exists, block until it is released.
    pub fn waiter<'fs>(&'fs self) -> FutureStreamWaiter<'fs, T> {
        let mut inner = self.inner.mx.lock().unwrap();

        loop {
            match inner.rx.take() {
                None => { inner = self.inner.cv.wait(inner).unwrap() },
                Some(rx) => return FutureStreamWaiter::new(self, rx),
            }
        }
    }

    /// Return a singleton `FutureStreamWaiter`. Returns `None` if one already exists.
    pub fn try_waiter<'fs>(&'fs self) -> Option<FutureStreamWaiter<'fs, T>> {
        let mut inner = self.inner.mx.lock().unwrap();

        match inner.rx.take() {
            None => None,
            Some(rx) => Some(FutureStreamWaiter::new(self, rx)),
        }
    }

    fn return_waiter(&self, rx: Receiver<Option<T>>) {
        let mut inner = self.inner.mx.lock().unwrap();

        assert!(inner.rx.is_none());
        inner.rx = Some(rx);
        self.inner.cv.notify_one();
    }

    /// Return a resolved `Future` if any, but don't wait for more to resolve.
    pub fn poll(&self) -> Option<Future<T>> {
        self.waiter().poll()
    }

    /// Return resolved `Future`s. Blocks if there are outstanding `Futures` which are not yet
    /// resolved. Returns `None` when there are no more outstanding `Future`s.
    pub fn wait(&self) -> Option<Future<T>> {
        self.waiter().wait()
    }
}

impl<'fs, T: Send> FutureStreamWaiter<'fs, T> {
    fn new(fs: &'fs FutureStream<T>, rx: Receiver<Option<T>>) -> FutureStreamWaiter<'fs, T> {
        FutureStreamWaiter { fs: fs, rx: Some(rx) }
    }

    /// Return resolved `Future`s. Blocks if there are outstanding `Futures` which are not yet
    /// resolved. Returns `None` when there are no more outstanding `Future`s.
    pub fn wait(&mut self) -> Option<Future<T>> {
        if { let l = self.fs.inner.mx.lock().unwrap(); l.pending == 0 } {
            // Nothing left
            None
        } else {
            // Wait for the next completion notification
            match self.rx.as_ref().unwrap().recv() {
                Ok(val) => {
                    let mut l = self.fs.inner.mx.lock().unwrap();
                    l.pending -= 1;
                    Some(Future::from(val))
                },
                Err(_) => None,
            }
        }
    }

    /// Return next resolved `Future`, but don't wait for more to resolve.
    pub fn poll(&mut self) -> Option<Future<T>> {
        let mut inner = self.fs.inner.mx.lock().unwrap();

        if inner.pending == 0 {
            None
        } else {
            match self.rx.as_ref().unwrap().try_recv() {
                Ok(val) => { inner.pending -= 1; Some(Future::from(val)) },
                Err(_) => None,
            }
        }
    }
}

impl<'fs, T: Send> Drop for FutureStreamWaiter<'fs, T> {
    fn drop(&mut self) {
        // Return notifications to FutureStream
        self.fs.return_waiter(self.rx.take().unwrap())
    }
}

/// Iterator for completed `Future`s in a `FutureStream`. The iterator incrementally returns values
/// from resolved `Future`s, blocking while there are no unresolved `Future`s. `Future`s which
/// resolve to no value are discarded.
pub struct FutureStreamIter<'a, T: Send + 'a>(FutureStreamWaiter<'a, T>);

impl<'fs, T: Send + 'fs> IntoIterator for FutureStreamWaiter<'fs, T> {
    type Item = T;
    type IntoIter = FutureStreamIter<'fs, T>;

    fn into_iter(self) -> Self::IntoIter { FutureStreamIter(self) }
}

impl<'a, T: Send + 'a> Iterator for FutureStreamIter<'a, T> {
    type Item = T;

    // Get next Future resolved with value, if any
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.0.wait() {
                None => return None,
                Some(fut) => {
                    match fut.poll() {
                        Unresolved(_) => panic!("FutureStreamWait.wait returned unresolved Future"),
                        Resolved(v@Some(_)) => return v,
                        Resolved(None) => (),
                    }
                },
            }
        }
    }
}

impl<'a, T: Send + 'a> IntoIterator for &'a FutureStream<T> {
    type Item = T;
    type IntoIter = FutureStreamIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter { self.waiter().into_iter() }
}

impl<T: Send + 'static> FromIterator<Future<T>> for FutureStream<T> {
    // XXX lazily consume input iterator?
    fn from_iter<I>(iterator: I) -> Self
        where I: IntoIterator<Item=Future<T>>
    {
        let stream = FutureStream::new();
        for f in iterator.into_iter() {
            stream.add(f)
        }

        stream
    }
}
