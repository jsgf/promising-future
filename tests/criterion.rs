#![cfg(feature = "criterion")]
#![feature(plugin)]
#![plugin(criterion_macros)]

extern crate criterion;
extern crate promising_future;
extern crate threadpool;

use std::mem;
use std::thread;
use criterion::Bencher;
use threadpool::ThreadPool;
use promising_future::{future_promise, Future};

#[criterion]
fn local_preset(b: &mut Bencher) {
    b.iter(|| {
        let f = Future::with_value(123);
        assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn local_never(b: &mut Bencher) {
    b.iter(|| {
        let f: Future<u32> = Future::never();
        assert_eq!(f.value(), None);
    })
}

#[criterion]
fn local_never_then(b: &mut Bencher) {
    b.iter(|| {
        let f: Future<u32> = Future::never();
        let f = f.then(|v| v);
        assert_eq!(f.value(), None);
    })
}

#[criterion]
fn local_preset_then(b: &mut Bencher) {
    b.iter(|| {
        let f = Future::with_value(123);
        let f = f.then(|v| v);
        assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn local(b: &mut Bencher) {
    b.iter_with_setup(future_promise,
        |(f, p)| {
            p.set(123);
            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn local_then(b: &mut Bencher) {
    b.iter_with_setup(future_promise,
        |(f, p)| {
            let f = f.then(|v| v);

            p.set(123);
            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn local_unfulfilled(b: &mut Bencher) {
    b.iter_with_setup(future_promise::<u32>,
        |(f, p)| {
            mem::drop(p);
            assert_eq!(f.value(), None);
    })
}

#[criterion]
fn thr_chain(b: &mut Bencher) {
    b.iter_with_setup(future_promise,
        |(f, p)| {
            let f = f.chain(|v| v);

            p.set(123);
            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn thr_from(b: &mut Bencher) {
    b.iter_with_setup(future_promise,
        |(f, p)| {
            thread::spawn(|| p.set(123));

            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn thr_chain_unfulfilled(b: &mut Bencher) {
    b.iter_with_setup(future_promise::<u32>,
        |(f, p)| {
            let f = f.chain(|v| v);

            mem::drop(p);
            assert_eq!(f.value(), None);
    })
}

#[criterion]
fn thrpool_from(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter_with_setup(future_promise,
        |(f, p)| {
            pool.execute(|| p.set(123));

            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn thrpool_chain(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter_with_setup(future_promise,
        |(f, p)| {
            let f = f.chain_with(|v| v, &pool);

            p.set(123);
            assert_eq!(f.value(), Some(123));
    })
}

#[criterion]
fn thrpool_chain_unfulfilled(b: &mut Bencher) {
    let pool = ThreadPool::new(2);

    b.iter_with_setup(future_promise::<u32>,
        |(f, p)| {
            let f = f.chain_with(|v| v, &pool);

            mem::drop(p);
            assert_eq!(f.value(), None);
    })
}
