//! Futures and Promises
//! ====================
//!
//! Quick example:
//!
//! ```
//! # use ::promising_future::future_promise;
//! # use std::time::Duration;
//! # #[allow(unused_variables)]
//! # use std::thread;
//! let (fut, prom) = future_promise();
//!
//! // A time-consuming process
//! thread::spawn(|| { thread::sleep(Duration::from_millis(100)); prom.set(123) });
//!
//! // do something when the value is ready
//! let fut = fut.then(|v| v + 1);
//!
//! // Wait for the final result
//! assert_eq!(fut.value(), Some(124));
//! ```
//!
//! This module implements a pair of concepts: `Future`s - a read-only placeholder for a variable
//! whose value may not yet be known, and `Promise`s - a write-once container which sets the
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
//! returns a `Future<Iterator<T>>`, so that its possible to wait for multiple Futures to be
//! resolved.
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
use std::iter::FromIterator;
use std::mem;
use std::thread;
use std::fmt::{self, Formatter, Debug};

mod fnbox;
use fnbox::{FnBox, Thunk};
mod futurestream;
use futurestream::WaiterNotify;
pub use futurestream::{FutureStream, FutureStreamIter, FutureStreamWaiter};

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
extern crate threadpool;

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

impl<T> Into<Option<T>> for Promiseval<T> {
    fn into(self) -> Option<T> {
        match self {
            Fulfilled(v) => Some(v),
            Unfulfilled => None
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
            Some(v) => Resolved(v.into()),
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

        val.take().expect("val None").into()
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
        where F: FnOnce(Option<T>) -> Option<U> + Send + 'static,
              U: Send + 'static
    {
        self.callback(move |v, p| if let Some(r) = func(v) { p.set(r) })
    }

    /// Set synchronous callback
    ///
    /// Simplest form of callback. This is only called if the promise
    /// is fulfilled, and may only allow a promise to be fulfilled.
    pub fn then<F, U>(self, func: F) -> Future<U>
        where F: FnOnce(T) -> U + Send + 'static,
              U: Send + 'static
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
        where F: FnOnce(Option<T>, Promise<U>) + Send + 'static,
              U: Send + 'static
    {
        let (fut, prom) = future_promise();

        // Lock order: Value, then...
        let mut val = self.mailbox.val.lock().unwrap();

        match val.take() {
            Some(v) => func(v.into(), prom), // already done, so apply callback now

            None => {
                // set function to be called in promise context
                match self.promise.and_then(|p| p.upgrade()) {
                    Some(cb) => {
                        // ...PromiseInner lock
                        let mut inner = cb.lock().unwrap();

                        inner.set_callback(move |v| func(v.into(), prom))
                    },
                    None => func(None, prom),      // no promise but awaiting value?
                }
            },
        };

        fut
    }

    // Called from FutureStream to add it as our waiter.
    fn add_waiter(&self, notify: WaiterNotify) {
        let p = self.promise.as_ref().and_then(|p| p.upgrade());
        match p {
            Some(mx) => {
                let mut lk = mx.lock().unwrap();
                lk.set_waiter(notify);
            },
            None => notify.notify(),    // no future, notify now
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
        waiter: Option<WaiterNotify>,
    },
    Callback(Thunk<'static, Promiseval<T>>),    // Callback which wants the value
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

    // Someone wants to know when this Promise is fulfilled.
    fn set_waiter(&mut self, notify: WaiterNotify) {
        match self {
            &mut PromiseInner::Future { ref mut waiter, .. } => {
                assert!(waiter.is_none());
                *waiter = Some(notify);
            },
            _ => notify.notify(),   // promise already set, just wake now
        }
    }

    fn canceled(&self) -> bool {
        match self {
            &PromiseInner::Empty => true,
            &PromiseInner::Callback(_) => false,
            &PromiseInner::Future { ref future, .. } => future.upgrade().is_none(), // canceled if we lost our Future
        }
    }

    // Return the innards, replacing them with Empty.
    fn get_future(&mut self) -> PromiseInner<T> {
        mem::replace(self, PromiseInner::Empty)
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

    // Set the value on the inner promise
    fn set_inner(&mut self, v: Promiseval<T>) {
        // Big retry loop
        loop {
            use PromiseInner::*;

            // The lock order is FutureInner then PromiseInner, but we need to take the PromiseInner
            // lock to find the Future, so first take the PromiseInner lock and get the Future if
            // there is one.
            let future = {
                let mut inner = self.0.lock().unwrap();
                inner.get_future()
            };

            // PromiseInner lock released; in this window Future::callback() could take the
            // PromiseInner lock and set a callback. We'll need to re-check once we re-take the
            // lock.
            match future {
                Future { future, waiter } => {
                    // If we have a Future then set the value, otherwise just dump it
                    if let Some(future) = future.upgrade() {
                        let mut flk = future.val.lock().unwrap(); // take Future value
                        let plk = self.0.lock().unwrap();

                        // We released PromiseInner lock, so the Future could have set a callback in the
                        // meantime. Make sure the state is still as we expect.
                        match *plk {
                            PromiseInner::Empty => (), // OK
                            PromiseInner::Callback(_) => continue, // it's a callback now; try again
                            PromiseInner::Future { .. } => panic!("PromiseInner regained a Future?"),
                        };

                        // OK, still a Future, set the value and wake up any waiters.
                        *flk = Some(v);
                        future.cv.notify_one();

                        // Send notification if anyone is waiting for that.
                        if let Some(notify) = waiter {
                            notify.notify()
                        }
                    }
                },

                Callback(cb) => cb.call_box(v), // no future, but there is a callback

                Empty => (),                    // Already set - used by Drop
            }

            // Finished
            break;
        }
    }

    /// Fulfill the `Promise` by resolving the corresponding `Future` with a value.
    pub fn set(mut self, v: T) {
        self.set_inner(Fulfilled(v))
    }

    /// Return true if the corresponding `Future` no longer exists, and so any value set would be
    /// discarded.
    ///
    /// ```
    /// # use ::promising_future::future_promise;
    /// # use std::thread;
    /// # use std::mem;
    /// # struct State; impl State { fn new() -> State { State } fn perform_action(&mut self) -> Option<u32> { None } }
    /// let (fut, prom) = future_promise();
    ///
    /// thread::spawn(move || {
    ///     let mut s = State::new();
    ///     while !prom.canceled() {
    ///         match s.perform_action() {
    ///             None => (),
    ///             Some(res) => { prom.set(res); break },
    ///         }
    ///     }
    /// });
    /// // ...
    /// mem::drop(fut);
    /// ```
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
/// let (fut, prom) = future_promise::<i32>();
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
    let stream = FutureStream::new();

    // XXX TODO need way to select on futures.into_iter() and stream.wait()...
    for fut in futures {
        // Check to see if future has already resolved; if it has a value return it immediately, or
        // discard it if it never will. Otherwise, if its unresolved, add it to the stream.
        match fut.poll() {
            Unresolved(fut) => stream.add(fut), // add to stream
            Resolved(v@Some(_)) => return v,    // return value
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
pub fn all_with<T, I, S, F>(futures: I, spawner: S) -> Future<F>
    where S: Spawner,
          T: Send + 'static,
          I: IntoIterator<Item=Future<T>> + Send + 'static,
          F: FromIterator<T> + Send + 'static
{
    let (f, p) = future_promise();
    let stream = FutureStream::from_iter(futures);

    spawner.spawn(move || {
        let waiter = stream.waiter();
        p.set(waiter.into_iter().collect());
    });

    f
}

/// Return a Future of all values in an iterator of `Future`s.
///
/// Take an iterator producing `Future<T>` values and return a `Future<Vec<T>>`.
///
/// This function is non-blocking; the blocking occurs within a thread. This uses
/// `std::thread::spawn()` to create the thread needed to block.
pub fn all<T, I, F>(futures: I) -> Future<F>
    where T: Send + 'static,
          I: IntoIterator<Item=Future<T>> + Send + 'static,
          F: FromIterator<T> + Send + 'static
{
    all_with(futures, ThreadSpawner)
}
