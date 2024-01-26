pub trait IsLast: Iterator + Sized {
    /// Returns an iterator that yields a tuple of (is_last, item).
    /// Note that this uses a peekable under the hood so items will be buffered.
    fn with_last(self) -> Iter<Self>;
}

impl<I> IsLast for I
where
    I: Iterator,
{
    fn with_last(self) -> Iter<Self> {
        Iter(self.peekable())
    }
}
pub struct Iter<I>(std::iter::Peekable<I>)
where
    I: Iterator;

impl<I> Iterator for Iter<I>
where
    I: Iterator,
{
    type Item = (bool, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|e| (self.0.peek().is_none(), e))
    }
}
