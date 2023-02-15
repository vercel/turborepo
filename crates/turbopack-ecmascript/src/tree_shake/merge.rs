pub trait Load {}

pub struct Merger<L> {
    loader: L,
}

impl<L> Merger<L> {}
