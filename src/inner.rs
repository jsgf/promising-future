use std::marker::PhantomData;
use crossbeam::Owned;
use fnbox::Thunk;

// Actual representation of inner - Enum tag stored in lower bits
struct UnsafeInner<T>(usize, PhantomData<T>);

impl UnsafeInner<T> {
    const EMPTY: usize = 0;
    const VALUE: usize = 1;
    const WAIT: usize = 2;
    const CALLBACK: usize = 3;
    
    unsafe fn empty() -> UnsafeInner<T> {
        UnsafeInner(EMPTY, PhantomData)
    }

    unsafe fn value(v: Box<T>) -> UnsafeInner<T> {
        let uv = v.into_raw() as usize;
        assert!(uv & 0x03 == 0);
        UnsafeInner(uv | VALUE, PhantomData)
    }

    unsafe fn wait(t: &Thread) -> UnsafeInner<T> {
        let ut = t as *mut Thread as usize;
        assert!(ut & 0x03 == 0);
        UnsafeInner(ut | WAIT, PhantomData)
    }

    unsafe fn callback(cb: Thunk<'static, T>) -> UnsafeInner<T> {
        let ucb = cb.into_raw() as usize;
        assert!(ucb & 0x03 == 0);
        UnsafeInner(ucb | CALLBACK, PhantomData)
    }
}

impl<T> From<UnsafeInner<T>> for Inner<T> {
    fn from(inner: UnsafeInner<T>) -> Self {
        match inner.0 & 0x03 {
            EMPTY => Inner::Empty,
            VALUE => {
                let raw = (inner.0 & !0x03) as *mut T;
                Inner::Value(Box::from_raw(raw))
            },
            WAIT => {
                
            }
        }
    }
}

pub enum Inner<T> {
    Empty,                      // No value, no waiter
    Value(Box<T>),              // has value
    Wait(Thread),               // Has waiter
    Callback(Thunk<'static,T>), // Has callback
}