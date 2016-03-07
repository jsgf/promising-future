use std::sync::Arc;
use std::mem;
use cvmx::CvMx;

#[derive(Debug, Copy, Clone)]
struct Val<T> {
    v: Option<T>,   // value, if set
    dead: bool,     // either post or mail gone
}

type Inner<T> = CvMx<Val<T>>;

#[derive(Debug)] pub struct Mailbox<T>(Arc<Inner<T>>);
#[derive(Debug)] pub struct Post<T>(Arc<Inner<T>>);

pub fn mailbox<T>() -> (Mailbox<T>, Post<T>) {
    let inner = Arc::new(CvMx::new(Val { v: None, dead: false }));
    let mail = Mailbox(inner.clone());
    let post = Post(inner);

    (mail, post)
}

#[derive(Debug, Eq, PartialEq)]
pub enum Pollresult {
    Value,
    Waiting,
    Dead,
}

impl<T> Mailbox<T> {
    /// Poll current state of the `Mailbox`.
    ///
    /// Return values are:
    ///
    /// * `Waiting` - waiting for a value
    /// * `Value` - have a values
    /// * `Dead` - don't have a value, and the corresponding `Post` is gone, so it will never have
    ///            a value.
    pub fn poll(&self) -> Pollresult {
        let inner = self.0.mx.lock().unwrap();

        match &inner.v {
            &None => if inner.dead { Pollresult::Dead } else { Pollresult::Waiting },
            &Some(_) => Pollresult::Value,
        }
    }

    /// Take the value from a `Mailbox`
    ///
    /// Take the value, if it has one. If it has a value, it consumes the `Mailbox` and Returns
    /// the value, otherwise it returns the `Mailbox` again.
    pub fn take(self) -> Result<T, Self> {
        let v = {
            let mut inner = self.0.mx.lock().unwrap();
            mem::replace(&mut inner.v, None)
        };
        match v {
            None => Err(self),
            Some(v) => Ok(v),
        }
    }

    /// Wait for a value in the `Mailbox`
    ///
    /// Returns `Ok` when it has a value, or `Err` if the `Post` has gone without posting a value.
    pub fn wait(self) -> Result<T, ()> {
        let mut inner = self.0.mx.lock().unwrap();

        while !inner.dead && inner.v.is_none() {
            inner = self.0.cv.wait(inner).unwrap()
        }

        match mem::replace(&mut inner.v, None) {
            None => { assert!(inner.dead); Err(()) },
            Some(v) => Ok(v),
        }
    }
}

impl<T> Drop for Mailbox<T> {
    fn drop(&mut self) {
        self.0.mx.lock().unwrap().dead = true;
    }
}

impl<T> Post<T> {
    /// Post a value to the corresponding `Mailbox`
    ///
    /// The value is lost if the `Mailbox` has gone.
    pub fn post(self, val: T) {
        let mut inner = self.0.mx.lock().unwrap();
        inner.v = Some(val);
        self.0.cv.notify_one();
    }

    /// Determine whether the `Mailbox` still exists.
    ///
    /// If this returns true, then any `post` will simply lose the value.
    pub fn isdead(&self) -> bool {
        self.0.mx.lock().unwrap().dead
    }
}

impl<T> Drop for Post<T> {
    fn drop(&mut self) {
        let mut inner = self.0.mx.lock().unwrap();
        inner.dead = true;
        self.0.cv.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;
    use std::mem;
    use std::sync::mpsc::channel;
    use super::*;

    fn sleep_ms(ms: u32) {
        thread::sleep(Duration::new(ms as u64 / 1000, (ms % 1000) * 1000_000))
    }

    #[test]
    fn basic() {
        {
            let (m, p) = mailbox();
            assert!(!p.isdead());
            assert_eq!(m.poll(), Pollresult::Waiting);
            p.post(123);
            assert_eq!(m.poll(), Pollresult::Value);
            assert_eq!(m.take().ok().unwrap(), 123);
        }
        {
            let (m, p) = mailbox();
            p.post(123);
            assert_eq!(m.wait().ok().unwrap(), 123);
        }
    }

    #[test]
    fn postdead() {
        {
            let (m, _) = mailbox::<()>();
            assert_eq!(m.poll(), Pollresult::Dead);
            assert!(m.take().is_err())
        }
        {
            let (m, _) = mailbox::<()>();
            assert_eq!(m.poll(), Pollresult::Dead);
            assert!(m.wait().is_err())
        }
    }

    #[test]
    fn boxdead() {
        let (_, p) = mailbox();
        assert!(p.isdead());
        p.post(123);
    }

    #[test]
    fn wait() {
        let (m, p) = mailbox();
        let (tx, rx) = channel();
        thread::spawn(move || { let _ = rx.recv(); sleep_ms(200); p.post(123) });
        assert_eq!(m.poll(), Pollresult::Waiting);
        let _ = tx.send(());
        assert_eq!(m.wait().ok().unwrap(), 123);
    }

    #[test]
    fn waitdead() {
        let (m, p) = mailbox::<u32>();
        let (tx, rx) = channel();
        thread::spawn(move || { let _ = rx.recv(); sleep_ms(200); mem::drop(p) });
        assert_eq!(m.poll(), Pollresult::Waiting);
        let _ = tx.send(());
        assert_eq!(m.wait().err().unwrap(), ());
    }
}
