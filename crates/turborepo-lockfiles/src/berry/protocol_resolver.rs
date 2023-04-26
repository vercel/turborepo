use std::collections::HashMap;

use super::identifiers::{Descriptor, Ident};

/// A data structure for resolving descriptors when the protocol isn't known
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DescriptorResolver<'a> {
    mapping: HashMap<Key<'a>, &'a str>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Key<'a> {
    ident: Ident<'a>,
    range: &'a str,
}

impl<'a> DescriptorResolver<'a> {
    /// Add a descriptor to the resolver
    pub fn insert(&mut self, descriptor: &Descriptor<'a>) -> Option<&'a str> {
        let key = Key::new(descriptor)?;
        self.mapping.insert(key, descriptor.range()?)
    }

    /// If given a descriptor without a protocol it will return all matching
    /// descriptors with a protocol
    pub fn get(&self, descriptor: &Descriptor) -> Option<&'a str> {
        let key = Key::new(descriptor)?;
        self.mapping.get(&key).copied()
    }
}

impl<'a> Key<'a> {
    fn new(desc: &Descriptor<'a>) -> Option<Self> {
        let ident = desc.ident.clone();
        let range = Descriptor::strip_protocol(desc.range()?);
        Some(Key { ident, range })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_descriptor_reconstruction() {
        let mut resolver = DescriptorResolver::default();
        let babel_npm = Descriptor::new("@babel/core", "npm:^5.0.0").unwrap();
        let babel_file = Descriptor::new("@babel/core", "file:4.5.0").unwrap();
        assert!(resolver.insert(&babel_npm).is_none());
        assert!(resolver.insert(&babel_file).is_none());
        let babel_5 = Descriptor::new("@babel/core", "^5.0.0").unwrap();
        let babel_4 = Descriptor::new("@babel/core", "4.5.0").unwrap();
        assert_eq!(resolver.get(&babel_5), Some("npm:^5.0.0"));
        assert_eq!(resolver.get(&babel_4), Some("file:4.5.0"));
    }

    #[test]
    fn test_descriptors_without_protocols() {
        let mut resolver = DescriptorResolver::default();
        let workspace = Descriptor::new("internal-workspace", "*").unwrap();
        assert!(resolver.insert(&workspace).is_none());
        assert_eq!(resolver.get(&workspace), Some("*"));
    }
}
