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

impl<T> Mailbox<T> {
    /// Take the value from a `Mailbox`
    ///
    /// Take the value, if it has one. It will only return the value once, and return
    /// and None thereafter.
    pub fn take(&mut self) -> Result<Option<T>, ()> {
        let mut inner = self.0.mx.lock().unwrap();

        match mem::replace(&mut inner.v, None) {
            None if inner.dead => Err(()),
            v => Ok(v)
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
    pub fn post(self, val: T) -> Result<(), T> {
        let mut inner = self.0.mx.lock().unwrap();

        if inner.dead {
            Err(val)
        } else {
            inner.v = Some(val);
            self.0.cv.notify_one();
            inner.dead = true;      // mark dead under lock
            Ok(())
        }
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
            let (mut m, p) = mailbox();
            assert!(!p.isdead());
            assert_eq!(m.take(), Ok(None));
            p.post(123).expect("post");
            assert_eq!(m.take(), Ok(Some(123)));
            assert_eq!(m.take(), Err(()));
        }
        {
            let (m, p) = mailbox();
            p.post(123).expect("post");
            assert_eq!(m.wait().ok().unwrap(), 123);
        }
    }

    #[test]
    fn postdead() {
        {
            let (mut m, _) = mailbox::<()>();
            assert!(m.take().is_err())
        }
        {
            let (m, _) = mailbox::<()>();
            assert!(m.wait().is_err())
        }
    }

    #[test]
    fn boxdead() {
        let (_, p) = mailbox();
        assert!(p.isdead());
        assert_eq!(p.post(123), Err(123))
    }

    #[test]
    fn wait() {
        let (mut m, p) = mailbox();
        let (tx, rx) = channel();
        thread::spawn(move || { let _ = rx.recv(); sleep_ms(200); p.post(123) });
        assert_eq!(m.take(), Ok(None));
        let _ = tx.send(());
        assert_eq!(m.wait().ok().unwrap(), 123);
    }

    #[test]
    fn waitdead() {
        let (mut m, p) = mailbox::<u32>();
        let (tx, rx) = channel();
        thread::spawn(move || { let _ = rx.recv(); sleep_ms(200); mem::drop(p) });
        assert_eq!(m.take(), Ok(None));
        let _ = tx.send(());
        assert_eq!(m.wait().err().unwrap(), ());
    }
}
