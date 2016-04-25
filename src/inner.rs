use std::thread::Thread;
use fnbox::Thunk;

pub enum Inner<T> {
    Empty,                       // No value, no waiter
    Value(T),                    // Has value
    Wait(Thread),                // Has waiter
    Callback(Thunk<'static, T>), // Has callback
    Dead,                        // No further use
}

impl<T> Inner<T> {
    pub fn empty() -> Self { Inner::Empty }
    pub fn value(v: T) -> Self { Inner::Value(v) }
    pub fn wait(th: Thread) -> Self { Inner::Wait(th) }
    pub fn callback(cb: Thunk<'static, T>) -> Self { Inner::Callback(cb) }
}