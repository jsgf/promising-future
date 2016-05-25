use std::fmt::{self, Formatter, Debug};

use fnbox::Thunk;

pub enum Inner<T> {
    Empty,                                  // No value yet
    Gone,                                   // Future has gone away, no point setting value
    Val(Option<T>),                         // Resolved value, if any
    Callback(Thunk<'static, Option<T>>),    // callback
}

impl<T> Inner<T> {
    pub fn new() -> Inner<T> { Inner::Empty }

    pub fn canceled(&self) -> bool {
        if let &Inner::Gone = self { true } else { false }
    }
}

impl<T: Debug> Debug for Inner<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use self::Inner::*;

        let state = match self {
            &Empty => "Empty".into(),
            &Val(ref v) => format!("Val({:?})", v),
            &Gone => "Gone".into(),
            &Callback(_) => "Callback(_)".into(),
        };
        write!(f, "{}", state)
    }
}