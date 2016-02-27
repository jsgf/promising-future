use std::iter::FromIterator;

use futurestream::FutureStream;
use spawner::{Spawner, ThreadSpawner};
use super::{future_promise, Future};

use Pollresult::*;

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
