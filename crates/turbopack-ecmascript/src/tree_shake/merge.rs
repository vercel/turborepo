pub trait Loader {}

pub struct Merger<L> {
    loader: L,
}

impl<L> Merger<L> {}
