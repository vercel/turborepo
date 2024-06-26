pub enum IntoIters4<A, B, C, D> {
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
    > Iterator for IntoIters4<A, B, C, D>
{
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIters4::One(iter) => iter.next(),
            IntoIters4::Two(iter) => iter.next(),
            IntoIters4::Three(iter) => iter.next(),
            IntoIters4::Four(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIters4::One(iter) => iter.size_hint(),
            IntoIters4::Two(iter) => iter.size_hint(),
            IntoIters4::Three(iter) => iter.size_hint(),
            IntoIters4::Four(iter) => iter.size_hint(),
        }
    }
}

pub enum IntoIters3<A, B, C> {
    One(A),
    Two(B),
    Three(C),
}

impl<I, A: Iterator<Item = I>, B: Iterator<Item = I>, C: Iterator<Item = I>> Iterator
    for IntoIters3<A, B, C>
{
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIters3::One(iter) => iter.next(),
            IntoIters3::Two(iter) => iter.next(),
            IntoIters3::Three(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIters3::One(iter) => iter.size_hint(),
            IntoIters3::Two(iter) => iter.size_hint(),
            IntoIters3::Three(iter) => iter.size_hint(),
        }
    }
}

pub enum IntoIters2<A, B> {
    One(A),
    Two(B),
}

impl<I, A: Iterator<Item = I>, B: Iterator<Item = I>> Iterator for IntoIters2<A, B> {
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IntoIters2::One(iter) => iter.next(),
            IntoIters2::Two(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            IntoIters2::One(iter) => iter.size_hint(),
            IntoIters2::Two(iter) => iter.size_hint(),
        }
    }
}
