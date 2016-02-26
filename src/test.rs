use super::*;
use super::Pollresult::{Resolved, Unresolved};
use std::thread;
use std::mem;
use std::iter::FromIterator;
use std::time::Duration;

// back compat
fn sleep_ms(ms: u32) {
    thread::sleep(Duration::new(ms as u64 / 1000, ((ms as u64 * 1000_000) % 1000_000_000) as u32))
}

#[test]
fn simple() {
    let (fut, prom) = future_promise();

    prom.set(1);
    assert_eq!(fut.value(), Some(1));
}

#[test]
fn poll() {
    let (fut, prom) = future_promise();

    let fut = match fut.poll() {
        Unresolved(s) => s,
        Resolved(_) => panic!("expected unresolved"),
    };

    prom.set(1);

    match fut.poll() {
        Unresolved(_) => panic!("expected resolved"),
        Resolved(v) => assert_eq!(v, Some(1)),
    }
}

#[test]
fn wait() {
    let (fut, prom) = future_promise();

    let t = thread::spawn(|| {
        sleep_ms(100);
        prom.set(1);
    });

    assert_eq!(fut.value(), Some(1));

    let _ = t.join();
}

#[test]
fn chain() {
    let (fut, prom) = future_promise();

    let fut = fut.chain(|x| x.map(|x| x + 1));
    let fut = fut.chain(|x| x.map(|x| x + 2));
    let fut = fut.chain(|x| x.map(|x| x + 3));

    sleep_ms(100);
    prom.set(1);

    assert_eq!(fut.value(), Some(7));
}

#[test]
fn chain_push() {
    let (fut, prom) = future_promise();

    prom.set(1);

    let fut = fut.chain(|x| x.map(|x| x + 1));
    let fut = fut.chain(|x| x.map(|x| x + 2));
    let fut = fut.chain(|x| x.map(|x| x + 3));

    assert_eq!(fut.value(), Some(7));
}

#[test]
fn testall() {
    let v = vec![Future::with_value(1), Future::with_value(2), Future::with_value(3)];

    assert_eq!(all(v).value(), Some(vec![1, 2, 3]));
}

#[test]
fn drop() {
    let (fut, prom) = future_promise();

    mem::drop(prom);

    assert_eq!(fut.value(), None::<i32>);
}

#[test]
fn drop_wait() {
    let (fut, prom) = future_promise();

    let t = thread::spawn(|| {
        sleep_ms(100);
        mem::drop(prom);
    });

    assert_eq!(fut.value(), None::<i32>);

    let _ = t.join();
}

#[test]
fn drop_chain() {
    let (fut, prom): (Future<i32>, _) = future_promise();

    let t = thread::spawn(|| {
        sleep_ms(100);
        mem::drop(prom);
    });

    let fut = fut.chain(|x| x.map(|x| x + 1));
    let fut = fut.chain(|x| x.map(|x| x + 2));
    let fut = fut.chain(|x| x.map(|x| x + 3));

    assert_eq!(fut.value(), None);

    let _ = t.join();
}

#[test]
fn never_chain() {
    let fut: Future<i32> = Future::never();

    let fut = fut.chain(|x| x.map(|x| x + 1));
    let fut = fut.chain(|x| x.map(|x| x + 2));
    let fut = fut.chain(|x| x.map(|x| x + 3));

    assert_eq!(fut.value(), None);
}

#[test]
fn never_all() {
    let v = vec![Future::with_value(1), Future::never(), Future::with_value(2), Future::never(), Future::with_value(3)];

    assert_eq!(all(v).value(), Some(vec![1, 2, 3]));
}

#[test]
fn all_wait() {
    let (fut, prom) = future_promise();
    let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

    let t = thread::spawn(|| {
        sleep_ms(100);
        prom.set(4);
    });

    assert_eq!(all(v).value(), Some(vec![1, 2, 3, 4]));

    let _ = t.join();
}

#[test]
fn all_drop_wait() {
    let (fut, prom) = future_promise();
    let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

    let t = thread::spawn(|| {
        sleep_ms(100);
        mem::drop(prom);
    });

    assert_eq!(all(v).value(), Some(vec![1, 2, 3]));

    let _ = t.join();
}

#[test]
fn then_simple() {
    let (fut, prom) = future_promise();

    let fut: Future<i32> = fut.then(|v| v);
    prom.set(1);
    assert_eq!(fut.value(), Some(1));
}

#[test]
fn then_drop() {
    let (fut, prom): (Future<i32>, _) = future_promise();

    let fut: Future<i32> = fut.then(|v| v);
    mem::drop(prom);
    assert_eq!(fut.value(), None);
}

#[test]
fn then_chain() {
    let (fut, prom): (Future<i32>, _) = future_promise();

    let fut = fut.then(|v| v + 1);
    let fut = fut.then(|v| v + 2);
    let fut = fut.then(|v| v + 3);

    prom.set(1);

    assert_eq!(fut.value(), Some(1 + 1 + 2 + 3));
}

#[test]
fn then_opt_chain() {
    let (fut, prom): (Future<i32>, _) = future_promise();

    let fut = fut.then_opt(|v| v.map(|v| v + 1));
    let fut = fut.then_opt(|_| None::<u32>);
    let fut = fut.then_opt(|v| v.map(|v| v + 3));

    prom.set(1);

    assert_eq!(fut.value(), None);
}

#[test]
fn canceled() {
    let (fut, prom): (Future<i32>, _) = future_promise();

    assert!(!prom.canceled());
    mem::drop(fut);
    assert!(prom.canceled());
}

#[test]
fn wait_any() {
    let mut v = Vec::new();
    for i in 1..10 {
        let (fut, prom) = future_promise();

        thread::spawn(move || { sleep_ms(i * 100); prom.set(i) });

        v.push(fut)
    }

    match any(v) {
        None => panic!("nothing!?"),
        Some(v) => {
            println!("got {:?}", v);
            assert_eq!(v, 1) // not really valid
        },
    }
}

#[test]
fn wait_any_abandoned() {
    let mut v = Vec::new();
    for i in 1..10 {
        let (fut, prom) = future_promise();

        thread::spawn(move || {
            sleep_ms(i * 100);
            if i >= 5 { prom.set(i) }
        });

        v.push(fut)
    }

    match any(v) {
        None => panic!("nothing!?"),
        Some(v) => {
            println!("got {:?}", v);
            assert_eq!(v, 5); // not really valid
        },
    }
}

#[test]
fn wait_all() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, prom) = future_promise();

        set.insert(i);
        thread::spawn(move || { sleep_ms(i * 100); prom.set(i) });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    let mut unfulfilled = 0;
    loop {
        match stream.wait() {
            None => break,
            Some(w) =>
                match w.poll() {
                    Unresolved(_) => panic!("unresolved future from wait"),
                    Resolved(None) => unfulfilled += 1,
                    Resolved(Some(w)) => {
                        set.remove(&w);
                        println!("got {:?}", w);
                    }
                }
        }
    }

    assert!(set.is_empty());
    assert_eq!(set.len(), unfulfilled);
}

#[test]
fn wait_abandoned() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, prom) = future_promise();

        set.insert(i);
        thread::spawn(move || {
            sleep_ms(i * 100);
            if i < 5 { prom.set(i) }
        });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    let mut unfulfilled = 0;
    loop {
        match stream.wait() {
            None => break,
            Some(w) =>
                match w.poll() {
                    Unresolved(_) => panic!("unresolved future from wait"),
                    Resolved(None) => unfulfilled += 1,
                    Resolved(Some(w)) => {
                        assert!(w < 5);
                        set.remove(&w);
                        println!("got {:?}", w);
                    }
                }
        }
    }

    assert_eq!(set.len(), 5);
    assert_eq!(set.len(), unfulfilled);
}

#[test]
fn wait_all_abandoned() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, _prom) = future_promise::<u32>();

        set.insert(i);
        thread::spawn(move || {
            //println!("spawn {}", i);
            sleep_ms(i * 100);
        });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    let mut unfulfilled = 0;
    loop {
        match stream.wait() {
            None => break,
            Some(w) =>
                match w.poll() {
                    Unresolved(_) => panic!("unresolved future from wait"),
                    Resolved(None) => unfulfilled += 1,
                    Resolved(Some(w)) => panic!("fulfilled promise {}", w),
                }
        }
    }

    assert_eq!(set.len(), 9);
    assert_eq!(set.len(), unfulfilled);
}

#[test]
fn wait_stream_all() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, prom) = future_promise();

        set.insert(i);
        thread::spawn(move || { sleep_ms(i * 100); prom.set(i) });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    for w in stream.waiter() {
        set.remove(&w);
        println!("got {:?}", w);
    }

    assert!(set.is_empty());
}

#[test]
fn wait_stream_abandoned() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, prom) = future_promise();

        set.insert(i);
        thread::spawn(move || {
            sleep_ms(i * 100);
            if i < 5 { prom.set(i) }
        });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    for w in stream.waiter() {
        assert!(w < 5);
        set.remove(&w);
        println!("got {}", w);
    }

    assert_eq!(set.len(), 5);
}

#[test]
fn wait_stream_all_abandoned() {
    use std::collections::BTreeSet;
    let mut v = Vec::new();
    let mut set = BTreeSet::new();

    for i in 1..10 {
        let (fut, _prom) = future_promise();

        set.insert(i);
        thread::spawn(move || {
            //println!("spawn {}", i);
            sleep_ms(i * 100);
        });

        v.push(fut)
    }

    let stream = FutureStream::from_iter(v);
    for w in stream.waiter() {
        set.remove(&w);
        println!("got {}", w);
    }

    assert_eq!(set.len(), 9);
}

#[test]
fn iter_none() {
    let v: Vec<()> = Future::never().into_iter().collect();
    assert_eq!(v.len(), 0);
}

#[test]
fn iter_one() {
    let v: Vec<()> = Future::with_value(()).into_iter().collect();
    assert_eq!(v.len(), 1);
}

#[test]
fn stress_fp() {
    const ITERS: u32 = 100000;
    let (futs, proms): (Vec<_>, Vec<_>) = (0..ITERS).into_iter().map(|i| { let (f, p) = future_promise(); ((i, f), (i, p)) }).unzip();

    let futurer = thread::spawn(move || {
        let mut count = 0;
        for (i, f) in futs.into_iter() {
            assert_eq!(f.value(), Some(i));
            count += 1;
        };
        count
    });

    let promiser = thread::spawn(move || {
        let mut count = 0;
        for (i, p) in proms.into_iter() {
            p.set(i);
            count += 1;
        }
        count
    });

    match futurer.join() {
        Ok(n) => assert_eq!(n, ITERS),
        Err(_) => panic!("futurer failed"),
    }
    match promiser.join() {
        Ok(n) => assert_eq!(n, ITERS),
        Err(_) => panic!("promiser failed"),
    }
}

#[test]
fn stress_fp_callback() {
    use std::sync::atomic::{ATOMIC_USIZE_INIT, Ordering};
    use std::sync::Arc;

    const ITERS: u32 = 100000;
    let (futs, proms): (Vec<_>, Vec<_>) = (0..ITERS).into_iter().map(|i| { let (f, p) = future_promise(); ((i, f), (i, p)) }).unzip();
    let futcount = Arc::new(ATOMIC_USIZE_INIT);

    let futurer = {
        let futcount = futcount.clone();

        thread::spawn(move || {
            for (idx, fut) in futs.into_iter() {
                let futcount = futcount.clone();

                let _: Future<()> = fut.callback(move |v, _| {
                    match v {
                        Some(i) => { assert_eq!(idx, i); futcount.fetch_add(1, Ordering::Relaxed); },
                        None => panic!("Lost value for idx {}", idx),
                    }
                });
            }
        })
    };

    let promiser = thread::spawn(move || {
        let mut count = 0;
        for (i, p) in proms.into_iter() {
            p.set(i);
            count += 1;
        }
        count
    });

    let _ = futurer.join();
    match promiser.join() {
        Ok(n) => assert_eq!(n, ITERS),
        Err(_) => panic!("promiser failed"),
    }

    assert_eq!(futcount.load(Ordering::Relaxed), ITERS as usize);
}

#[test]
fn stress_fp_waiter() {
    use std::collections::BTreeSet;

    const ITERS: u32 = 10000;
    let (futs, proms): (Vec<_>, Vec<_>) = (0..ITERS).into_iter().map(|i| { let (f, p) = future_promise(); (f, (i, p)) }).unzip();

    let promiser = thread::spawn(move || {
        let mut count = 0;
        for (i, p) in proms.into_iter() {
            p.set(i);
            count += 1;
        }
        count
    });

    let count = {
        let waiter = FutureStream::from_iter(futs);
        assert_eq!(waiter.outstanding(), ITERS as usize);
        let mut set: BTreeSet<_> = (0..ITERS).collect();

        let mut count = 0;
        for i in waiter.waiter().into_iter() {
            assert!(set.remove(&i));
            count += 1;
        }
        count
    };

    assert_eq!(count, ITERS);
    match promiser.join() {
        Ok(n) => assert_eq!(n, ITERS),
        Err(_) => panic!("promiser failed"),
    }
}

#[test]
fn sync_waiter() {
    const COUNT: u32 = 1000;
    let stream = FutureStream::new();
    let (f, p) = future_promise();

    stream.add(f);

    let wt = {
        let stream = stream.clone();

        thread::spawn(move || {
            let w = stream.waiter();

            let mut count = 0;
            for () in w.into_iter() {
                count += 1;
            }
            count
        })
    };

    let pt = {
        let stream = stream.clone();

        thread::spawn(move || {
            for _ in 0..COUNT {
                let (f, p) = future_promise();
                stream.add(f);
                p.set(());
                sleep_ms(2);
            }
        })
    };

    assert!(pt.join().is_ok());
    mem::drop(p);           // leave unfulfilled

    match wt.join() {
        Ok(n) => assert_eq!(n, COUNT),
        Err(e) => panic!("err {:?}", e),
    }
}

#[test]
fn double_waiter() {
    let fs: FutureStream<()> = FutureStream::new();

    {
        let _w1 = match fs.try_waiter() {
            None => panic!("couldn't get waiter"),
            Some(w) => w,
        };

        match fs.try_waiter() {
            None => (),
            Some(_) => panic!("got double waiter"),
        };
    }
}

#[cfg(feature = "threadpool")]
mod threadpool {
    use super::super::*;
    use threadpool::ThreadPool;
    use super::sleep_ms;

    #[test]
    fn tp_chain_with() {
        let pool = ThreadPool::new(5);
        let (fut, prom) = future_promise();

        let fut = fut.chain_with(|_| { sleep_ms(100); Some(()) }, &pool);
        prom.set(());
        assert_eq!(fut.value(), Some(()));
    }

    #[test]
    fn tp_all_with() {
        let pool = ThreadPool::new(5);
        let (fut, prom) = future_promise();
        let v = vec![Future::with_value(1), fut, Future::with_value(2), Future::never(), Future::with_value(3)];

        pool.execute(|| {
            sleep_ms(100);
            prom.set(4);
        });

        assert_eq!(all_with(v, pool).value(), Some(vec![1, 2, 3, 4]));
    }
}
