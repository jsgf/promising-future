#![feature(test)]

extern crate test;
extern crate promising_future;
extern crate threadpool;

use std::mem;
use std::thread;
use test::Bencher;
use threadpool::ThreadPool;
use promising_future::{future_promise, Future};

#[bench]
fn local_preset(b: &mut Bencher) {
    b.iter(|| {
        let f = Future::with_value(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn local_never(b: &mut Bencher) {
    b.iter(|| {
        let f: Future<u32> = Future::never();
        assert_eq!(f.value(), None);
    })
}

#[bench]
fn local_never_then(b: &mut Bencher) {
    b.iter(|| {
        let f: Future<u32> = Future::never();
        let f = f.then(|v| v);
        assert_eq!(f.value(), None);
    })
}

#[bench]
fn local_preset_then(b: &mut Bencher) {
    b.iter(|| {
        let f = Future::with_value(123);
        let f = f.then(|v| v);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn local(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise();
        p.set(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn local_then(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise();

        let f = f.then(|v| v);

        p.set(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn local_unfulfilled(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise::<u32>();
        mem::drop(p);
        assert_eq!(f.value(), None);
    })
}

#[bench]
fn thr_chain(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise();

        let f = f.chain(|v| v);

        p.set(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn thr_from(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise();

        thread::spawn(|| p.set(123));

        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn thr_chain_unfulfilled(b: &mut Bencher) {
    b.iter(|| {
        let (f, p) = future_promise::<u32>();

        let f = f.chain(|v| v);

        mem::drop(p);
        assert_eq!(f.value(), None);
    })
}

#[bench]
fn thrpool_from(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter(|| {
        let (f, p) = future_promise();

        pool.execute(|| p.set(123));

        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn thrpool_chain(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter(|| {
        let (f, p) = future_promise();

        let f = f.chain_with(|v| v, &pool);

        p.set(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[bench]
fn thrpool_chain_unfulfilled(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter(|| {
        let (f, p) = future_promise::<u32>();

        let f = f.chain_with(|v| v, &pool);

        mem::drop(p);
        assert_eq!(f.value(), None);
    })
}
