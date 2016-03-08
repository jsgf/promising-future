use std::fmt::{self, Debug};
use std::sync::mpsc::{Sender, Receiver, TryRecvError, RecvError, SendError, channel};

pub struct Mailbox<T>(Receiver<Option<T>>);
pub struct Post<T>(Sender<Option<T>>);

pub fn mailbox<T>() -> (Mailbox<T>, Post<T>) {
    let (tx, rx) = channel();

    (Mailbox(rx), Post(tx))
}

impl<T> Debug for Mailbox<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "Mailbox(...)")
    }
}

impl<T> Debug for Post<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "Post(...)")
    }
}

impl<T> Mailbox<T> {
    /// Take the value from a `Mailbox`
    ///
    /// Take the value, if it has one. It will only return the value once, and return
    /// and None thereafter.
    pub fn take(&mut self) -> Result<Option<T>, ()> {
        match self.0.try_recv() {
            Ok(v) => Ok(v),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }

    /// Wait for a value in the `Mailbox`
    ///
    /// Returns `Ok` when it has a value, or `Err` if the `Post` has gone without posting a value.
    pub fn wait(self) -> Result<T, ()> {
        loop {
            match self.0.recv() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(RecvError) => return Err(()),
            }
        }
    }
}

impl<T> Post<T> {
    /// Post a value to the corresponding `Mailbox`
    ///
    /// The value is lost if the `Mailbox` has gone.
    pub fn post(self, val: T) -> Result<(), T> {
        match self.0.send(Some(val)) {
            Ok(()) => Ok(()),
            Err(SendError(v)) => Err(v.unwrap()),
        }
    }

    /// Determine whether the `Mailbox` still exists.
    ///
    /// If this returns true, then any `post` will simply lose the value.
    pub fn isdead(&self) -> bool {
        self.0.send(None).is_err()
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
        {
            let (_, p) = mailbox::<u32>();
            assert!(p.isdead());
        }
        {
            let (_, p) = mailbox();
            assert_eq!(p.post(123), Err(123));
        }
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
