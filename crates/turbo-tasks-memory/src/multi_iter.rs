pub enum IntoIters<A, B, C, D> {
    One(A),
    Two(B),
    Three(C),
    Four(D),
}

impl<
        I,
        A: Iterator<Item = I>,
        B: Iterator<Item = I>,
        C: Iterator<Item = I>,
        D: Iterator<Item = I>,
    > Iterator for IntoIters<A, B, C, D>
{
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIters::One(iter) => iter.next(),
            IntoIters::Two(iter) => iter.next(),
            IntoIters::Three(iter) => iter.next(),
            IntoIters::Four(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIters::One(iter) => iter.size_hint(),
            IntoIters::Two(iter) => iter.size_hint(),
            IntoIters::Three(iter) => iter.size_hint(),
            IntoIters::Four(iter) => iter.size_hint(),
        }
    }
}
