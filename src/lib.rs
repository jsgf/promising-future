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

// Internal details
//
// A Future has a promise iff its Unresolved. A Future is either created resolved (`with_value`),
// in which case it never has a Promise, or becomes resolved by the Promise, which is destroyed
// in the process. A Promise can exist without a Future - for example, when a callback is set on
// the Future, which consumes the Future while setting the callback in the Promise.
//
// A Future can also be owned by a FutureStream, which prevents any other call on it. Therefore
// a Future in a FutureStream can't have a callback set on it.

/// An implementation of `Spawner` that spawns threads from a `ThreadPool`.
#[cfg(feature = "threadpool")]
extern crate threadpool;

use std::sync::Arc;
use std::mem;
use std::fmt::{self, Formatter, Debug};

mod fnbox;
mod spawner;
mod util;
mod futurestream;
mod cvmx;

use fnbox::{FnBox, Thunk};
use cvmx::CvMx;

pub use spawner::{Spawner, ThreadSpawner};
pub use util::{any, all, all_with};
pub use futurestream::{FutureStream, FutureStreamIter, FutureStreamWaiter};

/// Result of calling `Future.poll()`.
#[derive(Debug)]
pub enum Pollresult<T> {
    /// `Future` is not yet resolved; returns the `Future` for further use.
    Unresolved(Future<T>),

    /// `Future` has been resolved, and may or may not have a value. The `Future` has been consumed.
    Resolved(Option<T>),
}

#[doc(hidden)]
pub enum Inner<T> {
    Empty,                                  // No value yet
    Gone,                                   // Future has gone away, no point setting value
    Val(Option<T>),                         // Resolved value, if any
    Callback(Thunk<'static, Option<T>>),    // callback
}

impl<T> Inner<T> {
    fn new() -> Inner<T> { Inner::Empty }

    fn canceled(&self) -> bool {
        if let &Inner::Gone = self { true } else { false }
    }
}

impl<T: Debug> Debug for Inner<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use Inner::*;

        let state = match self {
            &Empty => "Empty".into(),
            &Val(ref v) => format!("Val({:?})", v),
            &Gone => "Gone".into(),
            &Callback(_) => "Callback(_)".into(),
        };
        write!(f, "{}", state)
    }
}

/// An undetermined value.
///
/// A `Future` represents an undetermined value. A corresponding `Promise` may set the value.
///
/// It is typically created in a pair with a `Promise` using the function `future_promise()`.
pub enum Future<T> {
    #[doc(hidden)]
    Const(Option<T>),
    #[doc(hidden)]
    Prom(Arc<CvMx<Inner<T>>>),
}

impl<T> Future<T> {
    fn new(inner: &Arc<CvMx<Inner<T>>>) -> Future<T> {
        Future::Prom(inner.clone())
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
        Future::Const(Some(v))
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
        Future::Const(None)
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
    pub fn poll(mut self) -> Pollresult<T> {
        use Future::*;
        use Inner::*;

        let res = match self {
            Const(ref mut v) => Some(v.take()),

            Prom(ref mut inner) => {
                let mut lk = inner.mx.lock().unwrap();

                match mem::replace(&mut *lk, Empty) {
                    Empty => None,
                    Val(v) => Some(v),
                    Callback(_) => panic!("callback in future"),
                    Gone => panic!("future gone"),
                }
            }
        };
        match res {
            Some(v) => Pollresult::Resolved(v),
            None => Pollresult::Unresolved(self),
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
    pub fn value(mut self) -> Option<T> {
        use Future::*;
        use Inner::*;

        match self {
            Const(ref mut v) => v.take(),
            Prom(ref inner) => {
                let mut lk = inner.mx.lock().unwrap();
                loop {
                    match mem::replace(&mut *lk, Empty) {
                        Empty => lk = inner.cv.wait(lk).expect("cv wait"),
                        Val(v) => return v,
                        Callback(_) => panic!("future with callback"),
                        Gone => panic!("future gone"),
                    }
                }
            },
        }
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
    /// all that's needed.
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

        let func = move |val| func(val, prom);

        self.callback_inner(func);

        fut
    }

    fn callback_inner<F>(mut self, func: F)
        where F: FnOnce(Option<T>) + Send + 'static
    {
        use Inner::*;

        match self {
            Future::Const(ref mut v) => func(v.take()),

            Future::Prom(ref mut inner) => {
                let mut lk = inner.mx.lock().unwrap();
                match mem::replace(&mut *lk, Empty) {
                    Empty => *lk = Callback(Box::new(func) as Thunk<'static, Option<T>>),
                    Val(v) => func(v),
                    Callback(_) => panic!("already have callback"),
                    Gone => panic!("future gone"),
                };
            },
        }
    }
}

impl<T: Send> Future<T> {
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
}

impl<T> Drop for Future<T> {
    fn drop(&mut self) {
        if let &mut Future::Prom(ref mut inner) = self {
            let mut lk = inner.mx.lock().expect("lock");

            if let &Inner::Empty = &*lk {
                *lk = Inner::Gone;
            }
        }
    }
}

impl<T> From<Option<T>> for Future<T> {
    fn from(v: Option<T>) -> Future<T> {
        match v {
            None => Future::never(),
            Some(v) => Future::with_value(v),
        }
    }
}

/// Blocking iterator for the value of a `Future`. Returns either 0 or 1 values.
pub struct FutureIter<T>(Option<Future<T>>);

impl<T> IntoIterator for Future<T> {
    type Item = T;
    type IntoIter = FutureIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        FutureIter(Some(self))
    }
}

impl<T> Iterator for FutureIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.take() {
            None => None,
            Some(fut) => fut.value()
        }
    }
}

impl<T: Debug> Debug for Future<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use Future::*;

        let state = match self {
            &Const(ref v) => format!("Const({:?})", v),
            &Prom(ref inner) => {
                let lk = inner.mx.lock().unwrap();
                format!("Prom({:?})", *lk)
            },
        };
        write!(f, "Future({}))", state)
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
pub struct Promise<T>(Arc<CvMx<Inner<T>>>);

impl<T> Promise<T> {
    fn new(inner: Arc<CvMx<Inner<T>>>) -> Promise<T> {
        Promise(inner)
    }

    // Set the value on the inner promise
    fn set_inner(&mut self, v: Option<T>) {
        use Inner::*;

        let mut lk = self.0.mx.lock().unwrap();

        match mem::replace(&mut *lk, Gone) {
            Gone => (),
            Empty => { *lk = Val(v); self.0.cv.notify_one() },
            v@Val(_) => *lk = v,            // we may get second set from Drop
            Callback(cb) => cb.call_box(v),
        }
    }

    /// Fulfill the `Promise` by resolving the corresponding `Future` with a value.
    pub fn set(mut self, v: T) {
        self.set_inner(Some(v))
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
        self.0.mx.lock().unwrap().canceled()
    }
}

impl<T> Drop for Promise<T> {
    fn drop(&mut self) {
        self.set_inner(None)
    }
}

impl<T: Debug> Debug for Promise<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Promise({:?})", *self.0.mx.lock().unwrap())
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
pub fn future_promise<T>() -> (Future<T>, Promise<T>) {
    let inner = Arc::new(CvMx::new(Inner::new()));
    let f = Future::new(&inner);
    let p = Promise::new(inner);

    (f, p)
}
