// FnBox workaround adapted from threadpool
pub trait FnBox<P> {
    fn call_box(self: Box<Self>, param: P);
}

impl<F: FnOnce(P), P> FnBox<P> for F {
    fn call_box(self: Box<F>, param: P) { (*self)(param) }
}

pub type Thunk<'a, P> = Box<FnBox<P> + Send + 'a>;
