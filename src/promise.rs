use std::fmt::{self, Formatter, Debug};
use std::sync::Arc;
use std::mem;

use cvmx::CvMx;
use inner::Inner;

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
    pub fn new(inner: Arc<CvMx<Inner<T>>>) -> Promise<T> {
        Promise(inner)
    }

    // Set the value on the inner promise
    fn set_inner(&mut self, v: Option<T>) {
        use inner::Inner::*;

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
