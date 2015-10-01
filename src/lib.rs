#![feature(fnbox)]
//! Futures and Promises
//! ====================
//!
//! Quick example:
//!
//! ```
//! # use ::promising_future::{future_promise};
//! # use std::thread;
//! let (fut, prom) = future_promise();
//!
//! // A time-consuming process
//! thread::spawn(|| { thread::sleep_ms(100); prom.set(123) });
//!
//! // do something when the value is ready
//! let fut = fut.then(|v| v + 1);
//!
//! // Wait for the final result
//! assert_eq!(fut.value(), Some(124));
//! ```
//!
//! This module implements a pair of concepts: `Future`s - a read-only placeholder for a variable
//! whose value may not yet be known, , and `Promise`s - a write-once container which sets the
//! value.
//!
//! A `Future` may either be "resolved" or "unresolved". An unresolved `Future` still has a pending
//! `Promise` for it. It becomes resolved once the `Promise` is complete. Once resolved, it will
//! have a value if the `Promise` was fulfilled (ie, set a value), or no value if the `Promise` was
//! unfulfilled (ie, dropped without setting a value).
//!
//! A `Promise` is either "pending" or "complete". A pending `Promise` is simply a live value of
//! type `Promise<T>`. It can be fulfilled by setting a value, which consumes the Promise,
//! completing it. Alternatively it can be completed unfulfilled by simply dropping the value
//! without ever calling `set` on it.
//!
//! A `Future` can also be created already resolved (ie, not paired with a `Promise`). This is
//! useful for lifting values into the `Promise`/`Future` domain.
//!
//! `Future`s may be chained in two ways. The most general way is with `callback`, which takes a
//! `Future` and a function to act on the value when it becomes available. This function is called
//! within the same context that completed the `Promise` so if the function blocks it will block
//! that context. The callback is passed another `Promise` to take the return of the callback, which
//! may be fulfilled or unfulfilled within the callback, or passed on somewhere else.
//!
//! Using `callback` directly can be a little cumbersome, so there are a couple of helpers. `then`
//! simply calls a synchronous callback function and uses its return to fulfill the value. The
//! function must be run within the `Promise` context, so it should probably be quick.
//!
//! Alternatively `chain` - like `then` - will take a function to act on the resolved future
//! value. However, unlike `then` it runs it in its own thread, so it can be arbitrarily
//! time-consuming. The variant `chain_with` allows the thread creation to be controlled, so that
//! thread pools may be used, for example.
//!
//! Groups of `Future`s can be acted upon together. `all` takes an iterator of `Future<T>`s, and
//! returns a `Future<Vec<T>>`, so that its possible to wait for multiple Futures to be resolved.
//!
//! Similarly, `any` returns the first available value of an iterator of `Future`s, discarding all
//! the other values.
//!
//! More generally, `FutureStream` provides a mechanism to wait on an arbitrary number of `Futures`
//! and incrementally acquiring their values as they become available.
//!
//! If a `Future` is dropped while its corresponding `Promise` is still pending, then any value it
//! does produce will be discarded. The `Promise` be queried with its `canceled` method to see if a
//! corresponding `Future` still exists; if not, it may choose to abort some time-consuming process
//! rather than have its output simply discarded.

use std::sync::{Mutex, Condvar, Arc, Weak};
use std::sync::mpsc::{Sender, Receiver, channel};
use std::iter::FromIterator;
use std::mem;
use std::thread;
use std::fmt::{self, Formatter, Debug};
use std::collections::HashMap;
use std::boxed::FnBox;

#[cfg(feature = "threadpool")]
extern crate threadpool;

/// A trait for spawning threads.
pub trait Spawner {
    /// Spawn a thread to run function `f`.
    fn spawn<F>(&self, f: F) where F: FnOnce() + Send + 'static;
}

/// An implementation of `Spawner` that creates normal `std::thread` threads.
pub struct ThreadSpawner;

impl Spawner for ThreadSpawner {
    fn spawn<F>(&self, f: F)
        where F: FnOnce() + Send + 'static
    {
        let _ = thread::spawn(f);
    }
}

/// An implementation of `Spawner` that spawns threads from a `ThreadPool`.
#[cfg(feature = "threadpool")]
impl Spawner for threadpool::ThreadPool {
    fn spawn<F>(&self, f: F)
        where F: FnOnce() + Send + 'static
    {
        self.execute(f)
    }
}

/// Result of calling `Future.poll()`.
#[derive(Debug)]
pub enum Pollresult<T: Send> {
    /// `Future` is not yet resolved; returns the `Future` for further use.
    Unresolved(Future<T>),

    /// `Future` has been resolved, and may or may not have a value. The `Future` has been consumed.
    Resolved(Option<T>),
}

use self::Pollresult::*;

#[derive(Debug)]
enum Promiseval<T> {
    Fulfilled(T),                   // value
    Unfulfilled,                    // Promise dropped without value
}

use self::Promiseval::*;

impl<T> Promiseval<T> {
    #[inline]
    fn into_option(self) -> Option<T> {
        match self {
            Fulfilled(v) => Some(v),
            Unfulfilled => None,
        }
    }
}

// Holder of the value being transferred from Promise to Future.
struct FutureInner<T> {
    cv: Condvar,
    val: Mutex<Option<Promiseval<T>>>
}

impl<T> FutureInner<T> {
    fn new(v: Option<Promiseval<T>>) -> FutureInner<T> {
        FutureInner {
            cv: Condvar::new(),
            val: Mutex::new(v),
        }
    }

    // Called from Promise when setting value
    fn set_val(&self, v: Promiseval<T>) {
        let mut flk = self.val.lock().unwrap();
        *flk = Some(v);
        self.cv.notify_one();
    }
}

impl<T: Debug> Debug for FutureInner<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "FutureInner {{ cv: .., val: {:?} }}", *self.val.lock().unwrap())
    }
}

/// An undetermined value.
///
/// A `Future` represents an undetermined value. A corresponding `Promise` may set the value.
///
/// It is typically created in a pair with a `Promise` using the function `future_promise()`.
pub struct Future<T: Send> {
    // Hold the value
    mailbox: Arc<FutureInner<T>>,

    // Back reference to Promise - if any - so that we can add a waiter
    promise: Option<Weak<Mutex<PromiseInner<T>>>>,
}

impl<T: Send> Future<T> {
    fn new(inner: Arc<FutureInner<T>>, promise: &Arc<Mutex<PromiseInner<T>>>) -> Future<T> {
        Future {
            mailbox: inner,
            promise: Some(Arc::downgrade(promise)),
        }
    }

    /// Construct an already resolved `Future` with a value. It is equivalent to a `Future` whose
    /// `Promise` has already been fulfilled.
    ///
    /// ```
    /// # use promising_future::{Future, Pollresult};
    /// let fut = Future::with_value(123);
    /// match fut.poll() {
    ///    Pollresult::Resolved(Some(123)) => println!("ok"),
    ///    _ => panic!("unexpected result"),
    /// }
    /// ```
    pub fn with_value(v: T) -> Future<T> {
        Future {
            mailbox: Arc::new(FutureInner::new(Some(Fulfilled(v)))),
            promise: None,
        }
    }

    /// Construct a resolved `Future` which will never have a value; it is equivalent to a `Future`
    /// whose `Promise` completed unfulfilled.
    ///
    /// ```
    /// # use promising_future::{Future, Pollresult};
    /// let fut = Future::<i32>::never();
    /// match fut.poll() {
    ///    Pollresult::Resolved(None) => println!("ok"),
    ///    _ => panic!("unexpected result"),
    /// }
    /// ```
    pub fn never() -> Future<T> {
        Future {
            mailbox: Arc::new(FutureInner::new(Some(Unfulfilled))),
            promise: None,
        }
    }

    /// Test to see if the `Future` is resolved yet.
    ///
    /// It returns an `Pollresult`, which has two values:
    ///
    /// * `Unresolved(Future<T>)` - the `Future` is not yet resolved, so returns itself, or
    /// * `Resolved(Option<T>)` - the `Future` has been resolved, and may have a value.
    ///
    /// ```
    /// # use promising_future::{Future, Pollresult};
    /// # let fut = Future::with_value(123);
    /// match fut.poll() {
    ///   Pollresult::Unresolved(fut) => println!("unresolved future {:?}", fut),
    ///   Pollresult::Resolved(None) => println!("resolved, no value"),
    ///   Pollresult::Resolved(Some(v)) => println!("resolved, value {}", v),
    /// }
    /// ```
    pub fn poll(self) -> Pollresult<T> {
        let val = self.mailbox.val.lock().unwrap().take();
        match val {
            None => Unresolved(self),
            Some(v) => Resolved(v.into_option()),
        }
    }

    /// Block until the `Future` is resolved.
    ///
    /// If the `Future` is not yet resolved, it will block until the corresponding `Promise` is
    /// either fulfilled, or is completed unfulfilled. In the former case it will return `Some(v)`,
    /// otherwise `None`.
    ///
    /// If the `Future` is already resolved - ie, has no corresponding `Promise` - then it will
    /// return immediately without blocking.
    ///
    /// ```
    /// # use promising_future::Future;
    /// # let fut = Future::with_value(123);
    /// match fut.value() {
    ///   Some(v) => println!("has value {}", v),
    ///   None => println!("no value"),
    /// }
    /// ```
    pub fn value(self) -> Option<T> {
        let mb = self.mailbox;
        let mut val = mb.val.lock().unwrap();

        while val.is_none() {
            val = mb.cv.wait(val).unwrap();
        }

        match val.take() {
            None => panic!("val None"),
            Some(v) => v.into_option(),
        }
    }

    /// Chain two `Future`s.
    ///
    /// Asynchronously apply a function to the result of a `Future`, returning a new `Future` for
    /// that value. This may spawn a thread to block waiting for the first `Future` to complete.
    ///
    /// The function is passed an `Option`, which indicates whether the `Future` ever received a
    /// value. The function returns an `Option` so that the resulting `Future` can also be
    /// valueless.
    #[inline]
    pub fn chain<F, U>(self, func: F) -> Future<U>
        where F: FnOnce(Option<T>) -> Option<U> + Send + 'static, T: 'static, U: Send + 'static
    {
        self.chain_with(func, &ThreadSpawner)
    }

    /// As with `chain`, but pass a `Spawner` to control how the thread is created.
    pub fn chain_with<F, U, S>(self, func: F, spawner: &S) -> Future<U>
        where F: FnOnce(Option<T>) -> Option<U> + Send + 'static, T: 'static, U: Send + 'static, S: Spawner
    {
        let (f, p) = future_promise();

        spawner.spawn(move || if let Some(r) = func(self.value()) { p.set(r) });

        f
    }

    /// Set a synchronous callback to run in the Promise's context.
    ///
    /// When the `Future<T>` completes, call the function on the value
    /// (if any), returning a new value which appears in the returned
    /// `Future<U>`.
    ///
    /// The function is called within the `Promise`s context, and so
    /// will block the thread if it takes a long time. Because the
    /// callback returns a value, not a `Future` it cannot be
    /// async. See `callback` or `chain` for more general async ways
    /// to apply a function to a `Future`.
    ///
    /// ```
    /// # use promising_future::future_promise;
    /// let (fut, prom) = future_promise();
    ///
    /// let fut = fut.then_opt(|v| v.map(|v| v + 123));
    /// prom.set(1);
    /// assert_eq!(fut.value(), Some(124))
    /// ```
    #[inline]
    pub fn then_opt<F, U>(self, func: F) -> Future<U>
        where F: FnOnce(Option<T>) -> Option<U> + Send + 'static, U: Send + 'static
    {
        self.callback(move |v, p| if let Some(r) = func(v) { p.set(r) })
    }

    /// Set synchronous callback
    ///
    /// Simplest form of callback. This is only called if the promise
    /// is fulfilled, and may only allow a promise to be fulfilled.
    pub fn then<F, U>(self, func: F) -> Future<U>
        where F: FnOnce(T) -> U + Send + 'static, U: Send + 'static
    {
        self.then_opt(move |v| v.map(func))
    }

    /// Set a callback to run in the `Promise`'s context.
    ///
    /// This function sets a general purpose callback which is called
    /// when a `Future` is resolved. It is called in the `Promise`'s
    /// context, so if it is long-running it will block whatever
    /// thread that is. (If the `Future` is already resolved, it is
    /// the calling thread.)
    ///
    /// The value passed to the callback is an `Option` - if it is
    /// `None` it means the promise was unfulfilled.
    ///
    /// The callback is passed a new `Promise<U>` which is paired with
    /// the `Future<U>` this function returns; the callback may either
    /// set a value on it, pass it somewhere else, or simply drop it
    /// leaving the promise unfulfilled.
    ///
    /// This is the most general form of a completion callback; see
    /// also `then` and `chain` for simpler interfaces which are often
    /// all that's needed..
    ///
    /// ```
    /// # use promising_future::future_promise;
    /// let (fut, prom) = future_promise();
    ///
    /// let fut = fut.callback(|v, p| {
    ///    match v {
    ///      None => (), // drop p
    ///      Some(v) => p.set(v + 123),
    ///    }
    /// });
    /// prom.set(1);
    /// assert_eq!(fut.value(), Some(124))
    /// ```
    pub fn callback<F, U>(self, func: F) -> Future<U>
        where F: FnOnce(Option<T>, Promise<U>) + Send + 'static, U: Send + 'static
    {
        let (fut, prom) = future_promise();

        match self.poll() {
            Resolved(v) => func(v, prom), // already done, so apply callback now

            Unresolved(me) => {
                // set function to be called in promise context
                match me.promise.and_then(|p| p.upgrade()) {
                    Some(cb) => {
                        let mut inner = cb.lock().unwrap();

                        inner.set_callback(move |v| func(v.into_option(), prom))
                    },
                    None => func(None, prom),      // no promise but awaiting value?
                }
            },
        };

        fut
    }

    // Called from FutureStream to add it as our waiter.
    fn add_waiter(&self, key: usize, waiter: Sender<usize>) {
        let p = self.promise.as_ref().and_then(|p| p.upgrade());
        match p {
            Some(mx) => {
                let mut lk = mx.lock().unwrap();
                lk.set_waiter(key, waiter);
            },
            None => { let _ = waiter.send(key); }, // wake immediately
        }
    }
}

impl<T: Send> From<Option<T>> for Future<T> {
    fn from(v: Option<T>) -> Future<T> {
        match v {
            None => Future::never(),
            Some(v) => Future::with_value(v),
        }
    }
}

/// Blocking iterator for the value of a `Future`. Returns either 0 or 1 values.
pub struct FutureIter<T: Send>(Option<Future<T>>);

impl<T: Send> IntoIterator for Future<T> {
    type Item = T;
    type IntoIter = FutureIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        FutureIter(Some(self))
    }
}

impl<T: Send> Iterator for FutureIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.take() {
            None => None,
            Some(fut) => fut.value()
        }
    }
}

impl<T: Send + Debug> Debug for Future<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Future {{ mailbox: {:?}, promise: {} }}", self.mailbox,
               match self.promise.as_ref().and_then(|p| p.upgrade()) {
                   None => From::from("None"),
                   Some(p) => format!("{:?}", *p.lock().unwrap()),
               })
    }
}

// Inner part of a Promise, which may also be weakly referenced by a Future.
enum PromiseInner<T: Send> {
    Empty,                      // no content
    Future {                    // future which will receive a value
        future: Weak<FutureInner<T>>,
        waiter: Option<(usize, Sender<usize>)>,
    },
    Callback(Box<FnBox(Promiseval<T>) + Send>),     // callback that will handle the value
}

impl<T: Send> PromiseInner<T> {
    // Create a Promise which holds a weak reference to its Future.
    fn with_future(fut: &Arc<FutureInner<T>>) -> PromiseInner<T> {
        PromiseInner::Future { future: Arc::downgrade(fut), waiter: None }
    }

    fn set_callback<F>(&mut self, cb: F)
        where F: FnOnce(Promiseval<T>) + Send + 'static
    {
        *self = PromiseInner::Callback(Box::new(cb))
    }

    fn set_waiter(&mut self, key: usize, waittx: Sender<usize>) {
        match self {
            &mut PromiseInner::Future { ref mut waiter, .. } => { assert!(waiter.is_none()); *waiter = Some((key, waittx)) },
            _ => {
                // promise already set, just wake now
                let _ = waittx.send(key);
            },
        }
    }

    fn canceled(&self) -> bool {
        match self {
            &PromiseInner::Empty => true,
            &PromiseInner::Callback(_) => false,
            &PromiseInner::Future { ref future, .. } => future.upgrade().is_none(), // canceled if we lost our Future
        }
    }

    fn set_val(&mut self, v: Promiseval<T>) {
        let mut current = PromiseInner::Empty;
        mem::swap(&mut current, self);

        match current {
            PromiseInner::Empty => (), // ignore if already set - used by Drop
            PromiseInner::Future { future, waiter } => {
                if let Some(future) = future.upgrade() {
                    future.set_val(v);
                }

                if let Some((idx, tx)) = waiter {
                    let _ = tx.send(idx);
                }
            },
            PromiseInner::Callback(cb) => cb(v),
        }
    }
}

impl<T: Send> Debug for PromiseInner<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            &PromiseInner::Empty => write!(f, "Empty"),
            &PromiseInner::Future { .. } => write!(f, "Future {{ .. }}"),
            &PromiseInner::Callback(_) => write!(f, "Callback"),
        }
    }
}

/// A box for resolving a `Future`.
///
/// A `Promise` is a write-once box which corresponds with a `Future` and may be used to resolve it.
///
/// A `Promise` is initially pending, and is completed once it is consumed, either by its `set`
/// method, or by going out of scope. The former is "fulfilling" the `Promise`; the latter is
/// leaving it "unfulfilled".
///
/// It may only be created in a pair with a `Future` using the function `future_promise()`.
pub struct Promise<T: Send>(Arc<Mutex<PromiseInner<T>>>);

impl<T: Send> Promise<T> {
    fn new(fut: &Arc<FutureInner<T>>) -> Promise<T> {
        Promise(Arc::new(Mutex::new(PromiseInner::with_future(fut))))
    }

    fn set_inner(&mut self, v: Promiseval<T>) {
        let mut inner = self.0.lock().unwrap();

        inner.set_val(v)
    }

    /// Fulfill the `Promise` by resolving the corresponding `Future` with a value.
    pub fn set(mut self, v: T) {
        self.set_inner(Fulfilled(v))
    }

    /// Return true if the corresponding `Future` no longer exists, and so any value set would be
    /// discarded.
    pub fn canceled(&self) -> bool {
        self.0.lock().unwrap().canceled()
    }
}

impl<T: Send> Drop for Promise<T> {
    fn drop(&mut self) {
        self.set_inner(Unfulfilled)
    }
}

impl<T: Send> Debug for Promise<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Promise({:?})", *self.0.lock().unwrap())
    }
}

/// Stream of multiple `Future`s
///
/// A `FutureStream` can be used to wait for multiple `Future`s, and return them incrementally as
/// they are resolved.
///
/// It implements an iterator over completed `Future`s, and can be constructed from an iterator of
/// `Future`s.
pub struct FutureStream<T: Send> {
    tx: Sender<usize>,
    rx: Receiver<usize>,
    idx: usize,
    futures: HashMap<usize, Future<T>>,
}

impl<T: Send> FutureStream<T> {
    pub fn new() -> FutureStream<T> {
        let (tx, rx) = channel();
        FutureStream {
            tx: tx,
            rx: rx,
            idx: 0,
            futures: HashMap::new(),
        }
    }

    /// Add a `Future` to the stream.
    pub fn add(&mut self, fut: Future<T>) {
        let idx = self.idx;
        self.idx += 1;
        let tx = self.tx.clone();
        fut.add_waiter(idx, tx);
        self.futures.insert(idx, fut);
    }

    /// Return number of outstanding `Future`s.
    pub fn outstanding(&self) -> usize {
        self.futures.len()
    }

    /// Return any resolved `Future`s, but don't wait for more to resolve.
    pub fn poll(&mut self) -> Option<Future<T>> {
        if self.futures.is_empty() {
            None
        } else {
            match self.rx.try_recv() {
                Ok(idx) => self.futures.remove(&idx),
                Err(_) => None,
            }
        }
    }

    /// Return resolved `Future`s. Blocks if there are outstanding `Futures` which are not yet
    /// resolved. Returns `None` when there are no more outstanding `Future`s.
    pub fn wait(&mut self) -> Option<Future<T>> {
        if self.futures.is_empty() {
            None
        } else {
            match self.rx.recv() {
                Ok(idx) => self.futures.remove(&idx),
                Err(_) => None,
            }
        }
    }
}

/// Iterator for completed `Future`s in a `FutureStream`. The iterator incrementally returns values
/// from resolved `Future`s, blocking while there are no unresolved `Future`s. `Future`s which
/// resolve to no value are discarded.
pub struct FutureStreamIter<T: Send>(FutureStream<T>);

impl<T: Send> IntoIterator for FutureStream<T> {
    type Item = T;
    type IntoIter = FutureStreamIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        FutureStreamIter(self)
    }
}

impl<T: Send> Iterator for FutureStreamIter<T> {
    type Item = T;

    // Get next Future resolved with value, if any
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.0.wait() {
                None => return None,
                Some(fut) => {
                    match fut.poll() {
                        Unresolved(_) => panic!("FutureStream.wait returned unresolved Future"),
                        Resolved(v@Some(_)) => return v,
                        Resolved(None) => (),
                    }
                },
            }
        }
    }
}

impl<T: Send> FromIterator<Future<T>> for FutureStream<T> {
    // XXX lazily consume input iterator?
    fn from_iter<I>(iterator: I) -> Self
        where I: IntoIterator<Item=Future<T>>
    {
        let mut stream = FutureStream::new();
        for f in iterator.into_iter() {
            stream.add(f)
        }

        stream
    }
}

/// Construct a `Future`/`Promise` pair.
///
/// A `Future` represents a value which may not yet be known. A `Promise` is some process which will
/// determine that value. This function produces a bound `Future`/`Promise` pair. If the `Promise`
/// is dropped before the value is set, then the `Future` will never return a value. If the `Future`
/// is dropped before fetching the value, or before the value is set, then the `Promise`'s value is
/// lost.
///
/// ```
/// # use promising_future::{Future, future_promise};
/// let (fut, prom): (Future<i32>, _) = future_promise();
/// ```
pub fn future_promise<T: Send>() -> (Future<T>, Promise<T>) {
    let inner = Arc::new(FutureInner::new(None));
    let p = Promise::new(&inner);
    let f = Future::new(inner, &p.0);

    (f, p)
}

/// Return first available `Future` from an iterator of `Future`s.
///
/// Given an iterator producing a stream of `Future<T>` values, return the first resolved value.
/// All other values are discarded.  `Future`s which resolve without values are ignored.
///
/// Note 1: This lazily consumes the input iterator so it can be infinite. However each unresolved `Future`
/// takes memory so `Future`s should resolve in bounded time (they need not resolve with values;
/// valueless `Future`s are discarded).
///
/// Note 2: `futures.into_iter()` should avoid blocking, as that will block this function even if
/// other `Future`s resolve. (FIXME)
pub fn any<T, I>(futures: I) -> Option<T>
    where I: IntoIterator<Item=Future<T>>, T: Send
{
    let mut stream = FutureStream::new();

    // XXX TODO need way to select on futures.into_iter() and stream.wait()...
    for fut in futures.into_iter() {
        // Check to see if future has already resolved; if it has a value return it immediately, or
        // discard it if it never will. Otherwise, if its unresolved, add it to the stream.
        match fut.poll() {
            Unresolved(fut) => stream.add(fut), // add to stream
            Resolved(v@Some(_)) => return v,      // return value
            Resolved(None) => (),               // skip
        };

        // Check to see if anything has become resolved
        while let Some(fut) = stream.poll() {
            match fut.poll() {
                Unresolved(_) => panic!("FutureStream.poll returned unresolved Future"),
                Resolved(v@Some(_)) => return v,
                Resolved(None) => (),
            }
        }
    }

    // Consumed whole input iterator, wait for something to finish
    while let Some(fut) = stream.wait() {
        match fut.poll() {
            Unresolved(_) => panic!("FutureStream.wait returned unresolved Future"),
            Resolved(v@Some(_)) => return v,
            Resolved(None) => (),
        }
    }
    None
}

/// Return a Future of all values in an iterator of `Future`s.
///
/// Take an iterator producing `Future<T>` values and return a `Future<Vec<T>>`. The elements in the
/// returned vector is undefined; typically it will be the order in which they resolved.
///
/// This function is non-blocking; the blocking occurs within a thread. Pass a type which implements
/// `Spawner` which is used to produce the thread.
pub fn all_with<T, I, S>(futures: I, spawner: S) -> Future<Vec<T>>
    where I: IntoIterator<Item=Future<T>> + Send + 'static, T: Send + 'static, S: Spawner
{
    let (f, p) = future_promise();
    let stream = FutureStream::from_iter(futures);

    spawner.spawn(move || p.set(stream.into_iter().collect()));

    f
}

/// Return a Future of all values in an iterator of `Future`s.
///
/// Take an iterator producing `Future<T>` values and return a `Future<Vec<T>>`.
///
/// This function is non-blocking; the blocking occurs within a thread. This uses
/// `std::thread::spawn()` to create the thread needed to block.
pub fn all<T, I>(futures: I) -> Future<Vec<T>>
    where I: IntoIterator<Item=Future<T>> + Send + 'static, T: Send + 'static
{
    all_with(futures, ThreadSpawner)
}

#[cfg(test)]
mod test {
    use super::*;
    use super::Pollresult::{Resolved, Unresolved};
    use std::thread;
    use std::mem;
    use std::iter::FromIterator;

    #[test]
    fn simple() {
        let (fut, prom) = future_promise();

        prom.set(1);
        assert_eq!(fut.value(), Some(1));
    }

    #[test]
    fn poll() {
        let (fut, prom) = future_promise();

        let fut = match fut.poll() {
            Unresolved(s) => s,
            Resolved(_) => panic!("expected unresolved"),
        };

        prom.set(1);

        match fut.poll() {
            Unresolved(_) => panic!("expected resolved"),
            Resolved(v) => assert_eq!(v, Some(1)),
        }
    }

    #[test]
    fn wait() {
        let (fut, prom) = future_promise();

        let t = thread::spawn(|| {
            thread::sleep_ms(100);
            prom.set(1);
        });

        assert_eq!(fut.value(), Some(1));

        let _ = t.join();
    }

    #[test]
    fn chain() {
        let (fut, prom) = future_promise();

        let fut = fut.chain(|x| x.map(|x| x + 1));
        let fut = fut.chain(|x| x.map(|x| x + 2));
        let fut = fut.chain(|x| x.map(|x| x + 3));

        thread::sleep_ms(100);
        prom.set(1);

        assert_eq!(fut.value(), Some(7));
    }

    #[test]
    fn chain_push() {
        let (fut, prom) = future_promise();

        prom.set(1);

        let fut = fut.chain(|x| x.map(|x| x + 1));
        let fut = fut.chain(|x| x.map(|x| x + 2));
        let fut = fut.chain(|x| x.map(|x| x + 3));

        assert_eq!(fut.value(), Some(7));
    }

    #[test]
    fn testall() {
        let v = vec![Future::with_value(1), Future::with_value(2), Future::with_value(3)];

        assert_eq!(all(v).value(), Some(vec![1, 2, 3]));
    }

    #[test]
    fn drop() {
        let (fut, prom) = future_promise();

        mem::drop(prom);

        assert_eq!(fut.value(), None::<i32>);
    }

    #[test]
    fn drop_wait() {
        let (fut, prom) = future_promise();

        let t = thread::spawn(|| {
            thread::sleep_ms(100);
            mem::drop(prom);
        });

        assert_eq!(fut.value(), None::<i32>);

        let _ = t.join();
    }

    #[test]
    fn drop_chain() {
        let (fut, prom): (Future<i32>, _) = future_promise();

        let t = thread::spawn(|| {
            thread::sleep_ms(100);
            mem::drop(prom);
        });

        let fut = fut.chain(|x| x.map(|x| x + 1));
        let fut = fut.chain(|x| x.map(|x| x + 2));
        let fut = fut.chain(|x| x.map(|x| x + 3));

        assert_eq!(fut.value(), None);

        let _ = t.join();
    }

    #[test]
    fn never_chain() {
        let fut: Future<i32> = Future::never();

        let fut = fut.chain(|x| x.map(|x| x + 1));
        let fut = fut.chain(|x| x.map(|x| x + 2));
        let fut = fut.chain(|x| x.map(|x| x + 3));

        assert_eq!(fut.value(), None);
    }

    #[test]
    fn never_all() {
        let v = vec![Future::with_value(1), Future::never(), Future::with_value(2), Future::never(), Future::with_value(3)];

        assert_eq!(all(v).value(), Some(vec![1, 2, 3]));
    }

    #[test]
    fn all_wait() {
        let (fut, prom) = future_promise();
        let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

        let t = thread::spawn(|| {
            thread::sleep_ms(100);
            prom.set(4);
        });

        assert_eq!(all(v).value(), Some(vec![1, 2, 3, 4]));

        let _ = t.join();
    }

    #[test]
    fn all_drop_wait() {
        let (fut, prom) = future_promise();
        let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

        let t = thread::spawn(|| {
            thread::sleep_ms(100);
            mem::drop(prom);
        });

        assert_eq!(all(v).value(), Some(vec![1, 2, 3]));

        let _ = t.join();
    }

    #[test]
    fn then_simple() {
        let (fut, prom) = future_promise();

        let fut: Future<i32> = fut.then(|v| v);
        prom.set(1);
        assert_eq!(fut.value(), Some(1));
    }

    #[test]
    fn then_drop() {
        let (fut, prom): (Future<i32>, _) = future_promise();

        let fut: Future<i32> = fut.then(|v| v);
        mem::drop(prom);
        assert_eq!(fut.value(), None);
    }

    #[test]
    fn then_chain() {
        let (fut, prom): (Future<i32>, _) = future_promise();

        let fut = fut.then(|v| v + 1);
        let fut = fut.then(|v| v + 2);
        let fut = fut.then(|v| v + 3);

        prom.set(1);

        assert_eq!(fut.value(), Some(1 + 1 + 2 + 3));
    }

    #[test]
    fn then_opt_chain() {
        let (fut, prom): (Future<i32>, _) = future_promise();

        let fut = fut.then_opt(|v| v.map(|v| v + 1));
        let fut = fut.then_opt(|_| None::<u32>);
        let fut = fut.then_opt(|v| v.map(|v| v + 3));

        prom.set(1);

        assert_eq!(fut.value(), None);
    }

    #[test]
    fn canceled() {
        let (fut, prom): (Future<i32>, _) = future_promise();

        assert!(!prom.canceled());
        mem::drop(fut);
        assert!(prom.canceled());
    }

    #[test]
    fn wait_any() {
        let mut v = Vec::new();
        for i in 1..10 {
            let (fut, prom) = future_promise();

            thread::spawn(move || { thread::sleep_ms(i * 100); prom.set(i) });

            v.push(fut)
        }

        match any(v) {
            None => panic!("nothing!?"),
            Some(v) => {
                println!("got {:?}", v);
                assert_eq!(v, 1) // not really valid
            },
        }
    }

    #[test]
    fn wait_any_abandoned() {
        let mut v = Vec::new();
        for i in 1..10 {
            let (fut, prom) = future_promise();

            thread::spawn(move || {
                thread::sleep_ms(i * 100);
                if i >= 5 { prom.set(i) }
            });

            v.push(fut)
        }

        match any(v) {
            None => panic!("nothing!?"),
            Some(v) => {
                println!("got {:?}", v);
                assert_eq!(v, 5); // not really valid
            },
        }
    }

    #[test]
    fn wait_all() {
        use std::collections::BTreeSet;
        let mut v = Vec::new();
        let mut set = BTreeSet::new();

        for i in 1..10 {
            let (fut, prom) = future_promise();

            set.insert(i);
            thread::spawn(move || { thread::sleep_ms(i * 100); prom.set(i) });

            v.push(fut)
        }

        for w in FutureStream::from_iter(v).into_iter() {
            set.remove(&w);
            println!("got {:?}", w);
        }

        assert!(set.is_empty());
    }

    #[test]
    fn wait_abandoned() {
        use std::collections::BTreeSet;
        let mut v = Vec::new();
        let mut set = BTreeSet::new();

        for i in 1..10 {
            let (fut, prom) = future_promise();

            set.insert(i);
            thread::spawn(move || {
                thread::sleep_ms(i * 100);
                if i < 5 { prom.set(i) }
            });

            v.push(fut)
        }

        for w in FutureStream::from_iter(v).into_iter() {
            assert!(w < 5);
            set.remove(&w);
            println!("got {}", w);
        }

        assert_eq!(set.len(), 5);
    }

    #[test]
    fn iter_none() {
        let v: Vec<()> = Future::never().into_iter().collect();
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn iter_one() {
        let v: Vec<()> = Future::with_value(()).into_iter().collect();
        assert_eq!(v.len(), 1);
    }

    #[cfg(feature = "threadpool")]
    mod threadpool {
        use super::super::*;
        use threadpool::ThreadPool;
        use std::thread;

        #[test]
        fn tp_chain_with() {
            let pool = ThreadPool::new(5);
            let (fut, prom) = future_promise();

            let fut = fut.chain_with(|_| { thread::sleep_ms(100); Some(()) }, &pool);
            prom.set(());
            assert_eq!(fut.value(), Some(()));
        }

        #[test]
        fn tp_all_with() {
            let pool = ThreadPool::new(5);
            let (fut, prom) = future_promise();
            let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

            pool.execute(|| {
                thread::sleep_ms(100);
                prom.set(4);
            });

            assert_eq!(all_with(v, pool).value(), Some(vec![1, 2, 3, 4]));
        }
    }
}
