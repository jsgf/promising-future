use std::thread;

#[cfg(feature = "threadpool")]
use threadpool;

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

#[cfg(feature = "threadpool")]
impl Spawner for threadpool::ThreadPool {
    fn spawn<F>(&self, f: F)
        where F: FnOnce() + Send + 'static
    {
        self.execute(f)
    }
}
