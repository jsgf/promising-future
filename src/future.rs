use std::fmt::{self, Formatter, Debug};
use std::sync::Arc;
use std::mem;

use cvmx::CvMx;
use inner::Inner;
use promise::Promise;
use fnbox::Thunk;
use spawner::{ThreadSpawner, Spawner};

/// Result of calling `Future.poll()`.
#[derive(Debug)]
pub enum Pollresult<T> {
    /// `Future` is not yet resolved; returns the `Future` for further use.
    Unresolved(Future<T>),

    /// `Future` has been resolved, and may or may not have a value. The `Future` has been consumed.
    Resolved(Option<T>),
}

/// An undetermined value.
///
/// A `Future` represents an undetermined value. A corresponding `Promise` may set the value.
///
/// It is typically created in a pair with a `Promise` using the function `future_promise()`.
pub enum Future<T> {
    Const(Option<T>),
    Prom(Arc<CvMx<Inner<T>>>),
}

impl<T> Future<T> {
    pub fn new(inner: &Arc<CvMx<Inner<T>>>) -> Future<T> {
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
    #[inline]
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
    #[inline]
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
        use self::Future::*;
        use inner::Inner::*;

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
        use self::Future::*;
        use inner::Inner::*;

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
    pub fn then_opt<F, U>(mut self, func: F) -> Future<U>
        where F: FnOnce(Option<T>) -> Option<U> + Send + 'static,
              U: Send + 'static
    {
        use self::Future::*;

        match self {
            Const(ref mut v) => {
                let v = mem::replace(v, None);
                Future::from(func(v))
            },
            Prom(_) => self.callback(move |v, p| if let Some(r) = func(v) { p.set(r) }),
        }
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

        self.callback_unit(func);

        fut
    }

    /// Set a callback which returns `()`
    ///
    /// Set a callback with a closure which returns nothing, so its only useful for its side-effects.
    pub fn callback_unit<F>(mut self, func: F)
        where F: FnOnce(Option<T>) + Send + 'static
    {
        use inner::Inner::*;

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

impl<T: Debug> Debug for Future<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use self::Future::*;

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

impl<T> From<Option<T>> for Future<T> {
    fn from(v: Option<T>) -> Future<T> {
        match v {
            None => Future::never(),
            Some(v) => Future::with_value(v),
        }
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
