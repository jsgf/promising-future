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

use std::sync::{Mutex, Arc, Weak};
use std::mem;
use std::fmt::{self, Formatter, Debug};

mod fnbox;
mod spawner;
mod util;
mod futurestream;
mod mailbox;
mod cvmx;

use fnbox::{FnBox, Thunk};
use mailbox::{mailbox, Mailbox, Post};
pub use spawner::{Spawner, ThreadSpawner};
pub use util::{any, all, all_with};
use futurestream::WaiterNotify;
pub use futurestream::{FutureStream, FutureStreamIter, FutureStreamWaiter};

/// Result of calling `Future.poll()`.
#[derive(Debug)]
pub enum Pollresult<T: Send> {
    /// `Future` is not yet resolved; returns the `Future` for further use.
    Unresolved(Future<T>),

    /// `Future` has been resolved, and may or may not have a value. The `Future` has been consumed.
    Resolved(Option<T>),
}

#[derive(Debug)]
enum FutureVal<T> {
    Empty,
    Val(T),
    Mailbox(Mailbox<T>),
}

/// An undetermined value.
///
/// A `Future` represents an undetermined value. A corresponding `Promise` may set the value.
///
/// It is typically created in a pair with a `Promise` using the function `future_promise()`.
pub struct Future<T: Send> {
    // Value from Promise, either constant or from promise
    val: FutureVal<T>,

    // Back reference to Promise - if any - so that we can set a callback.
    promise: Option<Weak<Mutex<PromiseInner<T>>>>,
    callback: Option<Post<Thunk<'static, Option<T>>>>,
}

impl<T: Send> Future<T> {
    fn new(mail: Mailbox<T>,
        promise: &Arc<Mutex<PromiseInner<T>>>,
        callback: Post<Thunk<'static, Option<T>>>) -> Future<T> {
        Future {
            val: FutureVal::Mailbox(mail),
            promise: Some(Arc::downgrade(promise)),
            callback: Some(callback),
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
            val: FutureVal::Val(v),
            promise: None,
            callback: None,
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
            val: FutureVal::Empty,
            promise: None,
            callback: None,
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
    pub fn poll(mut self) -> Pollresult<T> {
        use FutureVal::*;

        let val = mem::replace(&mut self.val, Empty);

        match val {
            Empty => Pollresult::Resolved(None),
            Val(v) => Pollresult::Resolved(Some(v)),
            Mailbox(mut mail) => {
                match mail.take() {
                    Ok(None) => {
                        self.val = Mailbox(mail);
                        Pollresult::Unresolved(self)
                    },
                    Ok(v@Some(_)) => Pollresult::Resolved(v),
                    Err(_) => Pollresult::Resolved(None),
                }
            },
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
        use FutureVal::*;

        match self.val {
            Empty => None,
            Val(v) => Some(v),
            Mailbox(mail) =>
                match mail.wait() {
                    Ok(v) => Some(v),
                    Err(_) => None,
                },
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
    pub fn callback<F, U>(mut self, func: F) -> Future<U>
        where F: FnOnce(Option<T>, Promise<U>) + Send + 'static,
              U: Send + 'static
    {
        use FutureVal::*;
        let (fut, prom) = future_promise();

        let func = move |val: Option<T>| func(val.into(), prom);

        match self.val {
            Empty => func(None),
            Val(v) => func(Some(v)),

            Mailbox(mut mail) => {
                // try posting func to other side
                let func = Box::new(func) as Box<FnBox<Option<T>> + Send>;
                let func =
                    match mem::replace(&mut self.callback, None) {
                        None => Some(func),
                        Some(cb) =>
                            match cb.post(func) {
                                Ok(_) => None,
                                Err(func) => Some(func),
                            },
                    };
                if let Some(func) = func {
                    // couldn't send it, handle locally
                    match mail.take() {
                        Err(_) => (),
                        Ok(v) => func.call_box(From::from(v)),
                    }
                }
            }
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
        use FutureVal::*;
        let state = match self.val {
            Empty => "Empty".into(),
            Val(_) => "Val(_)".into(),
            Mailbox(ref mb) => format!("Mailbox({:?})", mb),
        };

        write!(f, "Future {{ val: {},  }}", state)
    }
}

// Inner part of a Promise, which may also be weakly referenced by a Future.
enum PromiseInner<T: Send> {
    Empty,                                      // no content
    Future {                                    // future which will receive a value
        future: Post<T>,
        waiter: Option<WaiterNotify>,
    },
}

impl<T: Send> PromiseInner<T> {
    // Create a Promise which holds a weak reference to its Future.
    fn with_future(fut: Post<T>) -> PromiseInner<T> {
        PromiseInner::Future { future: fut, waiter: None }
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
            &PromiseInner::Future { ref future, .. } => future.isdead(),
        }
    }
}

impl<T: Send> Debug for PromiseInner<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            &PromiseInner::Empty => write!(f, "Empty"),
            &PromiseInner::Future { .. } => write!(f, "Future {{ .. }}"),
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
pub struct Promise<T: Send> {
    inner: Arc<Mutex<PromiseInner<T>>>,
    cbmail: Mailbox<Thunk<'static, Option<T>>>,
}

impl<T: Send> Promise<T> {
    fn new(fut: Post<T>, cbmail: Mailbox<Thunk<'static, Option<T>>>) -> Promise<T> {
        Promise {
            inner: Arc::new(Mutex::new(PromiseInner::with_future(fut))),
            cbmail: cbmail,
        }
    }

    // Set the value on the inner promise
    fn set_inner(&mut self, v: Option<T>) {
        use PromiseInner::*;

        // check for an existing callback and use it
        if let Ok(Some(cb)) = self.cbmail.take() {
            cb.call_box(v.into())
        } else {
            let mut inner = self.inner.lock().expect("inner lock");

            match mem::replace(&mut *inner, Empty) {
                Future { future, waiter } => {
                    if let Some(v) = v {
                        let _ = future.post(v); // discard value if future is gone
                    };
                    if let Some(notify) = waiter {
                        notify.notify();
                    };
                },

                Empty => (),
            };
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
        self.inner.lock().unwrap().canceled()
    }
}

impl<T: Send> Drop for Promise<T> {
    fn drop(&mut self) {
        self.set_inner(None)
    }
}

impl<T: Send> Debug for Promise<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Promise({:?})", *self.inner.lock().unwrap())
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
    let (mail, post) = mailbox();
    let (cbmail, cbpost) = mailbox();
    let p = Promise::new(post, cbmail);
    let f = Future::new(mail, &p.inner, cbpost);

    (f, p)
}
