use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Sender, Receiver, channel};
use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use std::iter::FromIterator;

use super::Pollresult::*;
use super::Future;

use cvmx::CvMx;

#[derive(Debug)]
struct TxCounts(AtomicUsize);
impl TxCounts {
    fn new() -> Self { TxCounts(ATOMIC_USIZE_INIT) }
}

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
    inner: Arc<FutureStreamInner<T>>,
}

struct FutureStreamInner<T: Send> {
    // Channel to notify complete Promises, which may be either resolved or
    // unresolved. Expectation is that every instance of tx will have a single
    // message sent.
    tx: Sender<Option<T>>,                      // endpoint to notify completion
    counts: Arc<TxCounts>,

    wait: CvMx<Option<FutureStreamWaitInner<T>>>,
}

impl<T: Send> FutureStreamInner<T> {
    fn new() -> Self {
        let (tx, rx) = channel();
        let counts = TxCounts::new();
        let inner = FutureStreamWaitInner::new(rx, &counts);

        FutureStreamInner {
            tx: tx,
            counts: counts,
            wait: CvMx::new(Some(inner)),
        }
    }
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
    inner: Option<Arc<FutureStreamWaitInner<T>>>,   // Option so that Drop can remove it
}

struct FutureStreamWaitInner<T: Send> {
    rx: Mutex<Receiver<Option<T>>>,             // index receiver
    counts: Arc<TxCounts>,
}
impl<T: Send> FutureStreamWaitInner<T> {
    fn new(rx: Receiver<Option<T>>, counts: &TxCounts) -> Self {
        FutureStreamWaitInner {
            rx: Mutex::new(rx),
            counts: counts.clone(),
        }
    }
}

impl<T: Send> FutureStream<T> {
    pub fn new() -> FutureStream<T> {
        let (tx, rx) = channel();
        let counts = Arc::new(TxCounts::new());

        let inner = FutureStreamWaitInner {
            rx: Mutex::new(rx),
            counts: counts.clone(),
        };

        FutureStream {
            tx: tx,
            counts: counts,
            inner: CvMx::new(Some(Arc::new(inner))),
        }
    }

    /// Add a `Future` to the stream.
    pub fn add(&self, fut: Future<T>)
        where T: 'static
    {
        let tx = self.tx.clone();
        self.counts.0.fetch_add(1, Ordering::Acquire);
        fut.inner_callback(move |v| { let _ = tx.send(v.into()); } );
    }

    /// Return number of outstanding `Future`s.
    pub fn outstanding(&self) -> usize {
        self.counts.0.load(Ordering::Relaxed)
    }

    /// Return a singleton `FutureStreamWaiter`. If one already exists, block until it is released.
    pub fn waiter<'fs>(&'fs self) -> FutureStreamWaiter<'fs, T> {
        let mut mx = self.inner.mx.lock().unwrap();

        loop {
            match mx.take() {
                None => { mx = self.inner.cv.wait(mx).unwrap() },
                Some(inner) => return FutureStreamWaiter::new(self, inner),
            }
        }
    }

    /// Return a singleton `FutureStreamWaiter`. Returns `None` if one already exists.
    pub fn try_waiter<'fs>(&'fs self) -> Option<FutureStreamWaiter<'fs, T>> {
        let mut mx = self.inner.mx.lock().unwrap();

        match mx.take() {
            None => None,
            Some(inner) => Some(FutureStreamWaiter::new(self, inner)),
        }
    }

    fn return_waiter(&self, inner: Arc<FutureStreamWaitInner<T>>) {
        let mut mx = self.inner.mx.lock().unwrap();

        assert!(mx.is_none());
        *mx = Some(inner);
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
    fn new(fs: &'fs FutureStream<T>, inner: Arc<FutureStreamWaitInner<T>>) -> FutureStreamWaiter<'fs, T> {
        FutureStreamWaiter { fs: fs, inner: Some(inner) }
    }

    // Count number of tx instances which could actually send
    fn num_tx(&self) -> usize {
        let counts = &self.inner.as_ref().unwrap().counts;
        let count = counts.0.load(Ordering::Relaxed);
        count
    }

    /// Return resolved `Future`s. Blocks if there are outstanding `Futures`
    /// which are not yet resolved. Returns `None` when there are no more
    /// outstanding `Future`s.
    pub fn wait(&mut self) -> Option<Future<T>> {
        let inner = &*self.inner.as_ref().unwrap();
        let rx = inner.rx.lock().unwrap();

        // poll first in case there's buffered results
        match rx.try_recv() {
            Ok(v) => return Some(Future::from(v)),
            Err(_) => (),
        };
        while self.num_tx() > 0 {
            match rx.recv() {
                Ok(v) => return Some(Future::from(v)),
                Err(_) => (),
            }
        }
        None
    }

    /// Return next resolved `Future`, but don't wait for more to resolve.
    pub fn poll(&mut self) -> Option<Future<T>> {
        match self.inner.as_ref().unwrap().rx.lock().unwrap().try_recv() {
            Ok(v) => Some(Future::from(v)),
            Err(_) => None,
        }
    }
}

impl<'fs, T: Send> Drop for FutureStreamWaiter<'fs, T> {
    fn drop(&mut self) {
        // Return notifications to FutureStream
        self.fs.return_waiter(self.inner.take().unwrap())
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

impl<T> FromIterator<Future<T>> for FutureStream<T>
    where T: Send + 'static
{
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
