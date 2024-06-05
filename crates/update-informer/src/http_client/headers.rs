use std::collections::{hash_map::IntoIter, HashMap};

#[derive(Default)]
/// A set of HTTP headers
pub struct HeaderMap<'a> {
    inner: HashMap<&'a str, &'a str>,
}

impl<'a> HeaderMap<'a> {
    pub(crate) fn new() -> Self {
        Default::default()
    }

    pub(crate) fn add(&mut self, key: &'a str, value: &'a str) {
        self.inner.insert(key, value);
    }
}

impl<'a> IntoIterator for HeaderMap<'a> {
    type Item = (&'a str, &'a str);
    type IntoIter = IntoIter<&'a str, &'a str>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}
