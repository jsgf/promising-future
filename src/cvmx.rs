use std::sync::{Mutex, Condvar};
use std::fmt::{self, Formatter, Debug};

// A Condvar and its Mutex
pub struct CvMx<T> {
    pub cv: Condvar,
    pub mx: Mutex<T>,
}

impl<T> CvMx<T> {
    pub fn new(v: T) -> CvMx<T> {
        CvMx {
            cv: Condvar::new(),
            mx: Mutex::new(v),
        }
    }
}

impl<T: Debug> Debug for CvMx<T> {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        write!(fmt, "CvMx({:?})", &*self.mx.lock().unwrap())
    }
}
